// Import resolution test fixture - type definitions
// Expected: Interface nodes for User and UserRole

export interface UserRole {
  id: number;
  name: string;
  permissions: string[];
}

export interface User {
  id: number;
  username: string;
  email: string;
  roles: UserRole[];
  createdAt: Date;
}
