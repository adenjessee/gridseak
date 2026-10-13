// Ground-truth oracle: every use of an in-repo function/method, resolved by
// the TypeScript checker (independent of SCIP / GridSeak).
const ts = require(process.env.TS_PATH);
const path = require('path');
const root = path.resolve(process.argv[2]);
const scope = path.join(root, process.argv[3] || 'src') + path.sep;
const cfgPath = ts.findConfigFile(root, ts.sys.fileExists, 'tsconfig.json');
const cfg = ts.getParsedCommandLineOfConfigFile(cfgPath, {}, { ...ts.sys, onUnRecoverableConfigFileDiagnostic: () => {} });
const program = ts.createProgram(cfg.fileNames, cfg.options);
const checker = program.getTypeChecker();
const rel = f => path.relative(root, f);
function calleeDecl(sym) {
  if (!sym) return null;
  if (sym.flags & ts.SymbolFlags.Alias) { try { sym = checker.getAliasedSymbol(sym); } catch { return null; } }
  const decls = [...(sym.declarations || [])].sort((a, b) => ((b.body ? 1 : 0) - (a.body ? 1 : 0)));
  for (const d of decls) {
    const sf = d.getSourceFile();
    if (!sf.fileName.startsWith(scope) || sf.isDeclarationFile) continue;
    let fnLike = ts.isFunctionDeclaration(d) || ts.isMethodDeclaration(d) || ts.isGetAccessor(d) || ts.isSetAccessor(d) || ts.isConstructorDeclaration(d);
    let iface = ts.isMethodSignature(d) || (ts.isMethodDeclaration(d) && d.body === undefined);
    if (ts.isVariableDeclaration(d) && d.initializer && (ts.isArrowFunction(d.initializer) || ts.isFunctionExpression(d.initializer))) fnLike = true;
    if (ts.isPropertyDeclaration(d) && d.initializer && (ts.isArrowFunction(d.initializer) || ts.isFunctionExpression(d.initializer))) fnLike = true;
    if (!fnLike && !iface) continue;
    const nameNode = d.name || d;
    const p = sf.getLineAndCharacterOfPosition(nameNode.getStart(sf));
    return { file: rel(sf.fileName), line: p.line + 1, name: sym.getName(), iface: iface && !fnLike, decl: d };
  }
  return null;
}
const out = []; const seen = new Set();
for (const sf of program.getSourceFiles()) {
  if (!sf.fileName.startsWith(scope) || sf.isDeclarationFile) continue;
  const visit = (n) => {
    if (ts.isIdentifier(n) || ts.isPrivateIdentifier(n)) {
      const parent = n.parent;
      const isDeclName = parent && parent.name === n && (ts.isFunctionDeclaration(parent) || ts.isMethodDeclaration(parent) || ts.isVariableDeclaration(parent) || ts.isPropertyDeclaration(parent) || ts.isMethodSignature(parent) || ts.isGetAccessor(parent) || ts.isSetAccessor(parent) || ts.isPropertyAssignment(parent));
      const inImport = ts.findAncestor(n, a => ts.isImportDeclaration(a) || ts.isExportDeclaration(a));
      if (!isDeclName && !inImport) {
        let sym = checker.getSymbolAtLocation(n);
        if (parent && ts.isShorthandPropertyAssignment(parent)) sym = checker.getShorthandAssignmentValueSymbol(parent);
        const d = calleeDecl(sym);
        if (d && !(d.decl.getStart() <= n.getStart() && n.getEnd() <= d.decl.getEnd() && d.decl.name === n)) {
          const p = sf.getLineAndCharacterOfPosition(n.getStart(sf));
          const isCall = !!parent && ((ts.isCallExpression(parent) && parent.expression === n) || (ts.isPropertyAccessExpression(parent) && parent.name === n && ts.isCallExpression(parent.parent) && parent.parent.expression === parent) || (ts.isNewExpression(parent) && parent.expression === n));
          const r = { ref_file: rel(sf.fileName), ref_line: p.line + 1, ref_col: p.character, callee_file: d.file, callee_line: d.line, callee_name: d.name, is_call: isCall, via_iface: d.iface };
          const enc = ts.findAncestor(n.parent, a => (ts.isFunctionDeclaration(a) || ts.isMethodDeclaration(a) || ts.isGetAccessor(a) || ts.isSetAccessor(a) || ts.isConstructorDeclaration(a) || ((ts.isArrowFunction(a) || ts.isFunctionExpression(a)) && (ts.isVariableDeclaration(a.parent) || ts.isPropertyDeclaration(a.parent) || ts.isPropertyAssignment(a.parent)))) && a !== d.decl);
          if (enc) { const nm = enc.name || (enc.parent && enc.parent.name) || enc; r.caller_file = rel(sf.fileName); r.caller_line = sf.getLineAndCharacterOfPosition(nm.getStart(sf)).line + 1; }
          const k = `${r.ref_file}:${r.ref_line}:${r.ref_col}`;
          if (!seen.has(k)) { seen.add(k); out.push(r); }
        }
      }
    }
    ts.forEachChild(n, visit);
  };
  visit(sf);
}
process.stdout.write(JSON.stringify(out));
