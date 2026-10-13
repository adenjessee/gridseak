// Single function test fixture
// Expected: One function node with FQN src/utils::calculateTotal

export function calculateTotal(items: number[]): number {
  return items.reduce((sum, item) => sum + item, 0);
}
