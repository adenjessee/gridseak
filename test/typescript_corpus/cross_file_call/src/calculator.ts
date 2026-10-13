// Cross-file call test fixture - consumer module
// Expected: Function node for 'calculate' with call edge to 'add' and 'multiply'

import { add, multiply } from './math';

export function calculate(a: number, b: number, c: number): number {
  const sum = add(a, b);
  const product = multiply(sum, c);
  return product;
}
