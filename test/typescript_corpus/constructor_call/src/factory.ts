// Constructor call test fixture
// Expected: Class nodes and call edges from factory function to constructors

export class Product {
  constructor(
    public id: number,
    public name: string,
    public price: number
  ) {}
}

export class Order {
  public items: Product[] = [];

  constructor(public orderId: string) {}

  addProduct(product: Product): void {
    this.items.push(product);
  }
}

// Factory function that creates instances
export function createOrder(orderId: string, products: Array<{ name: string; price: number }>): Order {
  const order = new Order(orderId);  // Constructor call
  
  products.forEach((p, index) => {
    const product = new Product(index, p.name, p.price);  // Constructor call
    order.addProduct(product);  // Method call
  });
  
  return order;
}
