// Cross-file call test fixture - utility module
// Expected: Function node for 'add'

export function add(a: number, b: number): number {
  return a + b;
}

export function multiply(a: number, b: number): number {
  return a * b;
}
