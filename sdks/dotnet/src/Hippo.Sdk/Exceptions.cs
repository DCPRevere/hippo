namespace Hippo.Sdk;

/// <summary>Base exception for all Hippo API errors.</summary>
public class HippoException : Exception
{
    public int StatusCode { get; }

    public HippoException(string message, int statusCode)
        : base(message)
    {
        StatusCode = statusCode;
    }

    public HippoException(string message, int statusCode, Exception innerException)
        : base(message, innerException)
    {
        StatusCode = statusCode;
    }
}

/// <summary>Thrown when the server returns 401 Unauthorized.</summary>
public class AuthenticationException : HippoException
{
    public AuthenticationException(string message)
        : base(message, 401) { }
}

/// <summary>Thrown when the server returns 403 Forbidden.</summary>
public class ForbiddenException : HippoException
{
    public ForbiddenException(string message)
        : base(message, 403) { }
}

/// <summary>Thrown when the server returns 429 Too Many Requests.</summary>
public class RateLimitException : HippoException
{
    /// <summary>Parsed value of the Retry-After header, if present.</summary>
    public TimeSpan? RetryAfter { get; }

    public RateLimitException(string message, TimeSpan? retryAfter = null)
        : base(message, 429)
    {
        RetryAfter = retryAfter;
    }
}
