// Ground-truth oracle: every reference from one in-repo function to another,
// resolved by go/types (independent of SCIP / GridSeak).
package main

import (
	"encoding/json"
	"fmt"
	"go/ast"
	"go/token"
	"go/types"
	"os"
	"path/filepath"
	"strings"

	"golang.org/x/tools/go/packages"
)

type Ref struct {
	CallerFile string `json:"caller_file"`
	CallerLine int    `json:"caller_line"`
	CallerName string `json:"caller_name"`
	CalleeFile string `json:"callee_file"`
	CalleeLine int    `json:"callee_line"`
	CalleeName string `json:"callee_name"`
	IsCall     bool   `json:"is_call"`
	ViaIface   bool   `json:"via_iface"`
}

func main() {
	root, _ := filepath.Abs(os.Args[1])
	// Optional ORACLE_TARGETS="rel/file.go:LINE,..." limits output to uses of those declarations.
	targets := map[string]bool{}
	for _, t := range strings.Split(os.Getenv("ORACLE_TARGETS"), ",") {
		if t != "" {
			targets[t] = true
		}
	}
	patterns := []string{"./..."}
	if p := os.Getenv("ORACLE_PATTERNS"); p != "" {
		patterns = strings.Split(p, " ")
	}
	cfg := &packages.Config{Mode: packages.LoadAllSyntax, Dir: root, Tests: true}
	pkgs, err := packages.Load(cfg, patterns...)
	if err != nil {
		panic(err)
	}
	seen := map[string]bool{}
	var out []Ref
	for _, p := range pkgs {
		fset := p.Fset
		for _, f := range p.Syntax {
			fname := fset.Position(f.Pos()).Filename
			if !strings.HasPrefix(fname, root) {
				continue
			}
			for _, d := range f.Decls {
				fd, ok := d.(*ast.FuncDecl)
				if !ok || fd.Body == nil {
					continue
				}
				callerLine := fset.Position(fd.Pos()).Line
				calls := map[*ast.Ident]bool{}
				ast.Inspect(fd.Body, func(n ast.Node) bool {
					if ce, ok := n.(*ast.CallExpr); ok {
						switch fn := ast.Unparen(ce.Fun).(type) {
						case *ast.Ident:
							calls[fn] = true
						case *ast.SelectorExpr:
							calls[fn.Sel] = true
						case *ast.IndexExpr:
							if id, ok := fn.X.(*ast.Ident); ok {
								calls[id] = true
							}
						}
					}
					return true
				})
				ast.Inspect(fd.Body, func(n ast.Node) bool {
					id, ok := n.(*ast.Ident)
					if !ok {
						return true
					}
					obj, ok := p.TypesInfo.Uses[id].(*types.Func)
					if !ok {
						return true
					}
					obj = obj.Origin()
					dpos := fset.Position(obj.Pos())
					if !dpos.IsValid() || !strings.HasPrefix(dpos.Filename, root) {
						return true
					}
					if len(targets) > 0 && !targets[fmt.Sprintf("%s:%d", rel(root, dpos.Filename), dpos.Line)] {
						return true
					}
					iface := false
					if sig, ok := obj.Type().(*types.Signature); ok && sig.Recv() != nil {
						if _, isI := sig.Recv().Type().Underlying().(*types.Interface); isI {
							iface = true
						}
					}
					r := Ref{
						CallerFile: rel(root, fname), CallerLine: callerLine, CallerName: fd.Name.Name,
						CalleeFile: rel(root, dpos.Filename), CalleeLine: dpos.Line, CalleeName: obj.Name(),
						IsCall: calls[id], ViaIface: iface,
					}
					k := fmt.Sprintf("%s:%d>%s:%d:%v", r.CallerFile, r.CallerLine, r.CalleeFile, r.CalleeLine, r.IsCall)
					if !seen[k] {
						seen[k] = true
						out = append(out, r)
					}
					return true
				})
			}
		}
	}
	_ = token.NoPos
	json.NewEncoder(os.Stdout).Encode(out)
}

func rel(root, p string) string { r, _ := filepath.Rel(root, p); return r }
