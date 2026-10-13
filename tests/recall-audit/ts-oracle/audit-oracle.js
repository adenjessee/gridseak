// Answer-key oracle for `gridseak audit` on TypeScript, built on the TypeScript
// checker (independent of SCIP / GridSeak). Extends oracle.js: records the use
// kind (call/new/super/tag/jsx/decorator/bind/value), resolves `new X()` and
// `super()` to the constructor that runs (inherited from the nearest base
// class when X declares none), treats bodiless (abstract/overload) methods as
// dispatch, and credits arrows in object-literal properties to the enclosing
// named function (set OBJ_ARROW_NAMED=1 to name them instead), matching the
// audit question. String-literal element accesses (`this["~validate"](x)`)
// are resolved through the checker like property accesses. Usage: TS_PATH=$(npm root -g)/typescript node audit-oracle.js <repo> [src]
// Check keys with ../ts_audit_keys.py.
const ts = require(process.env.TS_PATH);
const path = require('path');
const root = path.resolve(process.argv[2]);
const scope = path.join(root, process.argv[3] || 'src') + path.sep;
const cfgPath = ts.findConfigFile(root, ts.sys.fileExists, 'tsconfig.json');
const cfg = ts.getParsedCommandLineOfConfigFile(cfgPath, {}, { ...ts.sys, onUnRecoverableConfigFileDiagnostic: () => {} });
const program = ts.createProgram(cfg.fileNames, cfg.options);
const checker = program.getTypeChecker();
const rel = f => path.relative(root, f);
function declInfo(d, name) {
  const sf = d.getSourceFile();
  if (!sf.fileName.startsWith(scope) || sf.isDeclarationFile) return null;
  let fnLike = ((ts.isFunctionDeclaration(d) || ts.isMethodDeclaration(d)) && !!d.body) || ts.isGetAccessor(d) || ts.isSetAccessor(d) || ts.isConstructorDeclaration(d);
  let iface = ts.isMethodSignature(d) || (ts.isMethodDeclaration(d) && d.body === undefined) || (ts.isFunctionDeclaration(d) && d.body === undefined);
  if (ts.isVariableDeclaration(d) && d.initializer && (ts.isArrowFunction(d.initializer) || ts.isFunctionExpression(d.initializer))) fnLike = true;
  if (ts.isPropertyDeclaration(d) && d.initializer && (ts.isArrowFunction(d.initializer) || ts.isFunctionExpression(d.initializer))) fnLike = true;
  if (!fnLike && !iface) return null;
  const nameNode = ts.isConstructorDeclaration(d) ? (d.getChildren(sf).find(c => c.kind === ts.SyntaxKind.ConstructorKeyword) || d) : (d.name || d);
  const p = sf.getLineAndCharacterOfPosition(nameNode.getStart(sf));
  return { file: rel(sf.fileName), line: p.line + 1, name, iface: iface && !fnLike, decl: d, dkind: ts.SyntaxKind[d.kind] };
}
function calleeDecl(sym) {
  if (!sym) return null;
  if (sym.flags & ts.SymbolFlags.Alias) { try { sym = checker.getAliasedSymbol(sym); } catch { return null; } }
  const decls = [...(sym.declarations || [])].sort((a, b) => ((b.body ? 1 : 0) - (a.body ? 1 : 0)));
  for (const d of decls) {
    const r = declInfo(d, sym.getName());
    if (r) return r;
  }
  return null;
}
// Constructor `new C()` runs: C's own, else the nearest base class's (a class
// without one inherits its base's constructor).
function ctorOf(sym, depth = 0) {
  if (!sym || depth > 16) return null;
  if (sym.flags & ts.SymbolFlags.Alias) { try { sym = checker.getAliasedSymbol(sym); } catch { return null; } }
  if (!(sym.flags & ts.SymbolFlags.Class)) return null;
  for (const d of sym.declarations || []) {
    if (!ts.isClassDeclaration(d) && !ts.isClassExpression(d)) continue;
    const c = d.members.find(m => ts.isConstructorDeclaration(m) && m.body);
    if (c) return declInfo(c, 'constructor');
    const ext = (d.heritageClauses || []).find(h => h.token === ts.SyntaxKind.ExtendsKeyword);
    const base = ext && ext.types[0] && ext.types[0].expression;
    if (base) return ctorOf(checker.getSymbolAtLocation(ts.isPropertyAccessExpression(base) ? base.name : base), depth + 1);
  }
  return null;
}
function useKind(n) {
  const parent = n.parent;
  if (!parent) return 'value';
  let e = n;
  if (ts.isPropertyAccessExpression(parent) && parent.name === n) e = parent;
  const p = e.parent;
  if (!p) return 'value';
  if (ts.isCallExpression(p) && p.expression === e) return 'call';
  if (ts.isNewExpression(p) && p.expression === e) return 'new';
  if (ts.isTaggedTemplateExpression(p) && p.tag === e) return 'tag';
  if (ts.isDecorator(p)) return 'decorator';
  if ((ts.isJsxOpeningElement(p) || ts.isJsxSelfClosingElement(p) || ts.isJsxClosingElement(p)) && p.tagName === e) return 'jsx';
  if (ts.isNonNullExpression(p) && ts.isCallExpression(p.parent) && p.parent.expression === p) return 'call!';
  if (ts.isPropertyAccessExpression(p) && p.expression === e && ['call', 'apply', 'bind'].includes(p.name.text)) return p.name.text;
  return 'value';
}
function encOf(n, d) {
  return ts.findAncestor(n.parent, a => (ts.isFunctionDeclaration(a) || ts.isMethodDeclaration(a) || ts.isGetAccessor(a) || ts.isSetAccessor(a) || ts.isConstructorDeclaration(a) || ((ts.isArrowFunction(a) || ts.isFunctionExpression(a)) && (ts.isVariableDeclaration(a.parent) || ts.isPropertyDeclaration(a.parent) || (process.env.OBJ_ARROW_NAMED && ts.isPropertyAssignment(a.parent))))) && a !== d);
}
const out = []; const seen = new Set();
function emit(sf, n, d, kind) {
  const p = sf.getLineAndCharacterOfPosition(n.getStart(sf));
  const r = { ref_file: rel(sf.fileName), ref_line: p.line + 1, ref_col: p.character, callee_file: d.file, callee_line: d.line, callee_name: d.name, callee_dkind: d.dkind, kind, is_call: kind === 'call', via_iface: d.iface };
  const enc = encOf(n, d.decl);
  if (enc) {
    const nm = ts.isConstructorDeclaration(enc) ? (enc.getChildren(sf).find(c => c.kind === ts.SyntaxKind.ConstructorKeyword) || enc) : (enc.name || (enc.parent && enc.parent.name) || enc);
    r.caller_file = rel(sf.fileName); r.caller_line = sf.getLineAndCharacterOfPosition(nm.getStart(sf)).line + 1;
    r.caller_name = ts.isConstructorDeclaration(enc) ? 'constructor' : (nm.getText ? nm.getText(sf) : '?');
    r.caller_dkind = ts.SyntaxKind[enc.kind];
    r.caller_nested = !!ts.findAncestor(enc.parent, a => ts.isFunctionLike(a));
  }
  const k = `${r.ref_file}:${r.ref_line}:${r.ref_col}:${kind}`;
  if (!seen.has(k)) { seen.add(k); out.push(r); }
}
for (const sf of program.getSourceFiles()) {
  if (!sf.fileName.startsWith(scope) || sf.isDeclarationFile) continue;
  const visit = (n) => {
    if (ts.isCallExpression(n) && n.expression.kind === ts.SyntaxKind.SuperKeyword) {
      const sig = checker.getResolvedSignature(n);
      const dd = sig && sig.declaration;
      if (dd && ts.isConstructorDeclaration(dd) && dd.body) { const d = declInfo(dd, 'constructor'); if (d) emit(sf, n.expression, d, 'super'); }
    }
    // `obj["name"](…)`: a string-literal element access names a member just
    // like `obj.name`; the checker resolves the literal to that member.
    if (ts.isElementAccessExpression(n) && (ts.isStringLiteral(n.argumentExpression) || ts.isNoSubstitutionTemplateLiteral(n.argumentExpression))) {
      const sym = checker.getSymbolAtLocation(n.argumentExpression);
      const kind = useKind(n);
      const d = calleeDecl(sym);
      if (d) emit(sf, n.argumentExpression, d, kind);
    }
    if (ts.isIdentifier(n) || ts.isPrivateIdentifier(n)) {
      const parent = n.parent;
      const isDeclName = parent && parent.name === n && (ts.isFunctionDeclaration(parent) || ts.isMethodDeclaration(parent) || ts.isVariableDeclaration(parent) || ts.isPropertyDeclaration(parent) || ts.isMethodSignature(parent) || ts.isGetAccessor(parent) || ts.isSetAccessor(parent) || ts.isPropertyAssignment(parent));
      const inImport = ts.findAncestor(n, a => ts.isImportDeclaration(a) || ts.isExportDeclaration(a));
      if (!isDeclName && !inImport) {
        let sym = checker.getSymbolAtLocation(n);
        if (parent && ts.isShorthandPropertyAssignment(parent)) sym = checker.getShorthandAssignmentValueSymbol(parent);
        const kind = useKind(n);
        if (kind === 'new') { const c = ctorOf(sym); if (c) emit(sf, n, c, 'new'); }
        const d = calleeDecl(sym);
        if (d && !(d.decl.getStart() <= n.getStart() && n.getEnd() <= d.decl.getEnd() && d.decl.name === n)) emit(sf, n, d, kind);
      }
    }
    ts.forEachChild(n, visit);
  };
  visit(sf);
}
process.stdout.write(JSON.stringify(out));
