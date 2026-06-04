// Tiny TypeScript source used by the runner's parity + polyglot tests.
// Mirrors the shape of `lib.rs` so reports for the two languages are
// comparable (function + class with method + module-level export).

export function greet(name: string): string {
  return `hello ${name}`;
}

export class Counter {
  private count = 0;

  increment(): void {
    this.count += 1;
  }

  value(): number {
    return this.count;
  }
}

export const VERSION = '1.0.0';

export function repeatGreet(name: string, n: number): string[] {
  const out: string[] = [];
  for (let i = 0; i < n; i += 1) {
    out.push(greet(name));
  }
  return out;
}
