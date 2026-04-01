namespace Hippo.Sdk;

/// <summary>
/// Simple logging abstraction so callers can observe retry behaviour and request details
/// without pulling in an external dependency.
/// </summary>
public interface IHippoLogger
{
    void Debug(string message);
    void Warn(string message);
}
