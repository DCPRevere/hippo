export class HippoError extends Error {
  public readonly status: number;
  public readonly body: unknown;

  constructor(message: string, status: number, body?: unknown) {
    super(message);
    this.name = "HippoError";
    this.status = status;
    this.body = body;
  }
}

export class AuthenticationError extends HippoError {
  constructor(message = "Authentication failed", body?: unknown) {
    super(message, 401, body);
    this.name = "AuthenticationError";
  }
}

export class ForbiddenError extends HippoError {
  constructor(message = "Forbidden", body?: unknown) {
    super(message, 403, body);
    this.name = "ForbiddenError";
  }
}

export class RateLimitError extends HippoError {
  public readonly retryAfter: number | undefined;

  constructor(message = "Rate limit exceeded", body?: unknown, retryAfter?: number) {
    super(message, 429, body);
    this.name = "RateLimitError";
    this.retryAfter = retryAfter;
  }
}
