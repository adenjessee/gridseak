// Method call test fixture
// Expected: Class node (ApiService), method nodes, and call edges between methods

export class ApiService {
  private baseUrl: string;

  constructor(baseUrl: string) {
    this.baseUrl = baseUrl;
  }

  private buildUrl(endpoint: string): string {
    return `${this.baseUrl}/${endpoint}`;
  }

  async fetchData(endpoint: string): Promise<unknown> {
    const url = this.buildUrl(endpoint);  // Internal method call
    const response = await fetch(url);
    return response.json();
  }

  async getUserProfile(userId: number): Promise<unknown> {
    return this.fetchData(`users/${userId}`);  // Internal method call
  }
}
