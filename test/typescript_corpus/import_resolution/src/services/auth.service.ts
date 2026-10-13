// Import resolution test fixture - service using imported types
// Expected: Import edges from this file to types/user.ts

import type { User, UserRole } from '../types/user';

export class AuthService {
  private currentUser: User | null = null;

  login(username: string, password: string): Promise<User | null> {
    // Simulated login logic
    return Promise.resolve(null);
  }

  getCurrentUser(): User | null {
    return this.currentUser;
  }

  hasRole(role: string): boolean {
    if (!this.currentUser) return false;
    return this.currentUser.roles.some((r: UserRole) => r.name === role);
  }

  logout(): void {
    this.currentUser = null;
  }
}
