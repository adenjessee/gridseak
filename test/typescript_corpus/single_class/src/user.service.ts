// Single class test fixture
// Expected: One class node (UserService) and two method nodes (constructor, getUser)

export interface User {
  id: number;
  name: string;
  email: string;
}

export class UserService {
  private users: Map<number, User> = new Map();

  constructor() {
    // Initialize the service
  }

  getUser(id: number): User | undefined {
    return this.users.get(id);
  }

  createUser(name: string, email: string): User {
    const id = Date.now();
    const user: User = { id, name, email };
    this.users.set(id, user);
    return user;
  }
}
