// ifacegen: find interface-change edit tasks and the hidden compile-time
// assertions that grade "every implementation was updated".
//
// Embedded in `gridseak audit --family edits` (gridseak-cli/src/audit_edit.rs);
// derived from tests/difficulty/ifacegen. Read-only: both modes print JSON and
// never write into the repository.
//
//	ifacegen list   <repo> [patterns...]                  candidates as JSON
//	ifacegen assert <repo> <pkgpath> <Iface> [patterns...] {"files": {"rel/x_test.go": "source"}, "skipped": [...]}
package main

import (
	"encoding/json"
	"fmt"
	"go/types"
	"os"
	"path/filepath"
	"sort"
	"strings"

	"golang.org/x/tools/go/packages"
)

type Impl struct {
	Pkg     string `json:"pkg"`
	Dir     string `json:"dir"`
	Type    string `json:"type"`
	Pointer bool   `json:"pointer"`
	Test    bool   `json:"test"`
}

type Method struct {
	Name      string `json:"name"`
	Params    int    `json:"params"`
	CtxFirst  bool   `json:"ctx_first"`
	CallSites int    `json:"call_sites"`
}

type Cand struct {
	Pkg        string   `json:"pkg"`
	Dir        string   `json:"dir"`
	File       string   `json:"file"`
	Line       int      `json:"line"`
	Iface      string   `json:"iface"`
	Methods    []Method `json:"methods"`
	Impls      []Impl   `json:"impls"`
	CallerDirs []string `json:"caller_dirs"`
}

func load(root string, patterns []string) []*packages.Package {
	if len(patterns) == 0 {
		patterns = []string{"./..."}
	}
	cfg := &packages.Config{Mode: packages.LoadAllSyntax, Dir: root, Tests: true}
	pkgs, err := packages.Load(cfg, patterns...)
	if err != nil {
		fmt.Fprintln(os.Stderr, "ifacegen: load:", err)
		os.Exit(1)
	}
	return pkgs
}

func inRoot(root, f string) bool { return strings.HasPrefix(f, root+string(os.PathSeparator)) }

func main() {
	if len(os.Args) < 3 {
		fmt.Fprintln(os.Stderr, "usage: ifacegen list <repo> [patterns] | assert <repo> <pkgpath> <Iface> [patterns]")
		os.Exit(2)
	}
	mode := os.Args[1]
	root, _ := filepath.Abs(os.Args[2])
	if r, err := filepath.EvalSymlinks(root); err == nil {
		root = r
	}
	switch mode {
	case "list":
		list(root, os.Args[3:])
	case "assert":
		if len(os.Args) < 5 {
			fmt.Fprintln(os.Stderr, "usage: ifacegen assert <repo> <pkgpath> <Iface> [patterns]")
			os.Exit(2)
		}
		assert(root, os.Args[3], os.Args[4], os.Args[5:])
	default:
		fmt.Fprintln(os.Stderr, "unknown mode", mode)
		os.Exit(2)
	}
}

// Named types declared in repo files, with the package they live in and
// whether they are declared in a _test.go file.
func namedTypes(root string, pkgs []*packages.Package) map[*types.TypeName]Impl {
	out := map[*types.TypeName]Impl{}
	seenName := map[string]bool{}
	// Plain variants first so each type is checked against the plain
	// interface; test variants then only add types declared in _test.go.
	sort.SliceStable(pkgs, func(i, j int) bool { return pkgs[i].ID == pkgs[i].PkgPath && pkgs[j].ID != pkgs[j].PkgPath })
	for _, p := range pkgs {
		if p.TypesInfo == nil {
			continue
		}
		for id, obj := range p.TypesInfo.Defs {
			tn, ok := obj.(*types.TypeName)
			if !ok || tn.Parent() != p.Types.Scope() {
				continue
			}
			f := p.Fset.Position(id.Pos()).Filename
			if !inRoot(root, f) {
				continue
			}
			if _, isIface := tn.Type().Underlying().(*types.Interface); isIface {
				continue
			}
			if n, ok := tn.Type().(*types.Named); ok && n.TypeParams().Len() > 0 {
				continue
			}
			pkgPath := strings.TrimSuffix(p.PkgPath, "_test")
			isTest := strings.HasSuffix(f, "_test.go")
			nameKey := pkgPath + "." + tn.Name()
			if _, seen := out[tn]; seen || seenName[nameKey] || (p.ID != p.PkgPath && !isTest) {
				continue
			}
			seenName[nameKey] = true
			out[tn] = Impl{Pkg: pkgPath, Dir: rel(root, filepath.Dir(f)), Type: tn.Name(), Test: isTest}
		}
	}
	return out
}

func list(root string, patterns []string) {
	pkgs := load(root, patterns)
	named := namedTypes(root, pkgs)
	var cands []*Cand
	seen := map[string]bool{}
	for _, p := range pkgs {
		if p.TypesInfo == nil {
			continue
		}
		for id, obj := range p.TypesInfo.Defs {
			tn, ok := obj.(*types.TypeName)
			if !ok || tn.Parent() != p.Types.Scope() || !tn.Exported() {
				continue
			}
			if n, ok := tn.Type().(*types.Named); ok && n.TypeParams().Len() > 0 {
				continue
			}
			it, ok := tn.Type().Underlying().(*types.Interface)
			if !ok || it.NumMethods() == 0 || it.NumMethods() > 4 || it.NumEmbeddeds() > 0 {
				continue
			}
			pos := p.Fset.Position(id.Pos())
			key := p.PkgPath + "." + tn.Name()
			if !inRoot(root, pos.Filename) || seen[key] || strings.HasSuffix(pos.Filename, "_test.go") {
				continue
			}
			seen[key] = true
			c := &Cand{
				Pkg: strings.TrimSuffix(p.PkgPath, "_test"), Dir: rel(root, filepath.Dir(pos.Filename)),
				File: rel(root, pos.Filename), Line: pos.Line, Iface: tn.Name(),
			}
			for i := 0; i < it.NumMethods(); i++ {
				m := it.Method(i)
				sig := m.Type().(*types.Signature)
				ctx := sig.Params().Len() > 0 && types.TypeString(sig.Params().At(0).Type(), nil) == "context.Context"
				c.Methods = append(c.Methods, Method{Name: m.Name(), Params: sig.Params().Len(), CtxFirst: ctx})
			}
			for t, info := range named {
				switch {
				case types.Implements(t.Type(), it):
					c.Impls = append(c.Impls, info)
				case types.Implements(types.NewPointer(t.Type()), it):
					info.Pointer = true
					c.Impls = append(c.Impls, info)
				}
			}
			sort.Slice(c.Impls, func(i, j int) bool { return c.Impls[i].Pkg+c.Impls[i].Type < c.Impls[j].Pkg+c.Impls[j].Type })
			cands = append(cands, c)
		}
	}
	// Count call sites through each interface method, and the directories
	// they appear in. Every type-check variant of the interface's package
	// has its own *types.Func, so map them all.
	type ref struct {
		c *Cand
		m int
	}
	byMethod := map[*types.Func]ref{}
	for _, c := range cands {
		for _, p := range pkgs {
			if strings.TrimSuffix(p.PkgPath, "_test") != c.Pkg || p.Types == nil {
				continue
			}
			obj := p.Types.Scope().Lookup(c.Iface)
			if obj == nil {
				continue
			}
			it, ok := obj.Type().Underlying().(*types.Interface)
			if !ok {
				continue
			}
			for k := 0; k < it.NumMethods(); k++ {
				for mi := range c.Methods {
					if c.Methods[mi].Name == it.Method(k).Name() {
						byMethod[it.Method(k)] = ref{c, mi}
					}
				}
			}
		}
	}
	callerDirs := map[*Cand]map[string]bool{}
	seenSite := map[string]bool{}
	for _, p := range pkgs {
		if p.TypesInfo == nil {
			continue
		}
		for expr, sel := range p.TypesInfo.Selections {
			fn, ok := sel.Obj().(*types.Func)
			if !ok {
				continue
			}
			r, ok := byMethod[fn]
			if !ok {
				continue
			}
			pos := p.Fset.Position(expr.Pos())
			if !inRoot(root, pos.Filename) {
				continue
			}
			site := fmt.Sprintf("%p|%s:%d:%d", r.c, pos.Filename, pos.Line, pos.Column)
			if seenSite[site] {
				continue
			}
			seenSite[site] = true
			r.c.Methods[r.m].CallSites++
			if callerDirs[r.c] == nil {
				callerDirs[r.c] = map[string]bool{}
			}
			callerDirs[r.c][rel(root, filepath.Dir(pos.Filename))] = true
		}
	}
	out := []Cand{}
	for _, c := range cands {
		for d := range callerDirs[c] {
			c.CallerDirs = append(c.CallerDirs, d)
		}
		sort.Strings(c.CallerDirs)
		out = append(out, *c)
	}
	sort.Slice(out, func(i, j int) bool { return out[i].Pkg+"."+out[i].Iface < out[j].Pkg+"."+out[j].Iface })
	json.NewEncoder(os.Stdout).Encode(out)
}

func assert(root, pkgPath, iface string, patterns []string) {
	pkgs := load(root, patterns)
	named := namedTypes(root, pkgs)
	// Use the plain (non-test) variant: test variants re-type-check the
	// package, so their interface is not identical to the one other
	// packages implement.
	var it *types.Interface
	for _, p := range pkgs {
		if p.PkgPath == pkgPath && p.ID == p.PkgPath && p.Types != nil {
			if obj := p.Types.Scope().Lookup(iface); obj != nil {
				it, _ = obj.Type().Underlying().(*types.Interface)
			}
		}
	}
	if it == nil {
		fmt.Fprintln(os.Stderr, "ifacegen: interface not found:", pkgPath+"."+iface)
		os.Exit(1)
	}
	// Plain package variants by path, to detect import cycles below.
	byPath := map[string]*packages.Package{}
	for _, p := range pkgs {
		if p.ID == p.PkgPath {
			byPath[p.PkgPath] = p
		}
	}
	files := map[string][]string{}
	pkgName := map[string]string{}
	implImport := map[string]string{}
	skipped := []string{}
	for t, info := range named {
		ptr := false
		switch {
		case types.Implements(t.Type(), it):
		case types.Implements(types.NewPointer(t.Type()), it):
			ptr = true
		default:
			continue
		}
		name := t.Name()
		pkg := t.Pkg().Name()
		// Always a _test.go file, which is never linked into another
		// package. An internal test of a package that the interface's
		// package itself imports would still form a cycle when it imports
		// the interface's package; assert exported types from an external
		// `_test` package instead, and skip the rest (reported).
		if t.Pkg().Path() != pkgPath && !strings.HasSuffix(t.Pkg().Path(), "_test") && reaches(byPath, pkgPath, t.Pkg().Path()) {
			if !t.Exported() || info.Test {
				skipped = append(skipped, info.Pkg+"."+t.Name())
				continue
			}
			name = "ifacegen_impl." + t.Name()
			pkg = t.Pkg().Name() + "_test"
			implImport[info.Dir+"|"+pkg] = t.Pkg().Path()
		}
		expr := name + "{}"
		if ptr {
			expr = "(*" + name + ")(nil)"
		}
		if _, isStruct := t.Type().Underlying().(*types.Struct); !isStruct && !ptr {
			expr = "(" + name + ")(*new(" + name + "))"
		}
		qual := "ifacegen_iface." + iface
		if t.Pkg().Path() == pkgPath && !strings.HasSuffix(pkg, "_test") {
			qual = iface
		}
		key := info.Dir + "|" + pkg
		files[key] = append(files[key], fmt.Sprintf("var _ %s = %s", qual, expr))
		pkgName[key] = pkg
	}
	out := map[string]string{}
	for key, lines := range files {
		dir := strings.SplitN(key, "|", 2)[0]
		sort.Strings(lines)
		body := strings.Join(lines, "\n")
		var imps []string
		if strings.Contains(body, "ifacegen_iface.") {
			imps = append(imps, fmt.Sprintf("import ifacegen_iface %q", pkgPath))
		}
		if p, ok := implImport[key]; ok {
			imps = append(imps, fmt.Sprintf("import ifacegen_impl %q", p))
		}
		imp := ""
		if len(imps) > 0 {
			imp = strings.Join(imps, "\n") + "\n\n"
		}
		name := "zz_ifacegen_assert_test.go"
		if strings.HasSuffix(pkgName[key], "_test") {
			name = "zz_ifacegen_assert_x_test.go"
		}
		out[filepath.ToSlash(filepath.Join(dir, name))] = fmt.Sprintf("package %s\n\n%s%s\n", pkgName[key], imp, body)
	}
	sort.Strings(skipped)
	json.NewEncoder(os.Stdout).Encode(map[string]any{"files": out, "skipped": skipped})
}

// reaches reports whether package `from` imports `to`, directly or not.
func reaches(byPath map[string]*packages.Package, from, to string) bool {
	seen := map[string]bool{}
	var walk func(string) bool
	walk = func(p string) bool {
		if p == to {
			return true
		}
		if seen[p] {
			return false
		}
		seen[p] = true
		pk := byPath[p]
		if pk == nil {
			return false
		}
		for ip := range pk.Imports {
			if walk(ip) {
				return true
			}
		}
		return false
	}
	return walk(from)
}

func rel(root, p string) string {
	if r, err := filepath.EvalSymlinks(p); err == nil {
		p = r
	}
	r, _ := filepath.Rel(root, p)
	return filepath.ToSlash(r)
}
