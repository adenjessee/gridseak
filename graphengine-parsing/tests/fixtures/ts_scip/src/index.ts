export function callee(): number {
  return 1;
}

export function caller(): number {
  return callee();
}

export class Parser {
  parse(): string {
    return "ok";
  }
}

export function useParser(obj: Parser): string {
  return obj.parse();
}
