namespace Hippo.Sdk;

/// <summary>
/// Convenience extension methods for Hippo response models.
/// </summary>
public static class ResponseExtensions
{
    /// <summary>Find the first fact whose subject matches <paramref name="name"/> (case-insensitive).</summary>
    public static ContextFact? FindSubject(this ContextResponse response, string name) =>
        response.Facts.FirstOrDefault(f =>
            string.Equals(f.Subject, name, StringComparison.OrdinalIgnoreCase));

    /// <summary>Return facts where the entity appears as subject or object (case-insensitive).</summary>
    public static IEnumerable<ContextFact> FactsAbout(this ContextResponse response, string entityName) =>
        response.Facts.Where(f =>
            string.Equals(f.Subject, entityName, StringComparison.OrdinalIgnoreCase) ||
            string.Equals(f.Object, entityName, StringComparison.OrdinalIgnoreCase));

    /// <summary>True when nothing new was written (the statement was already known).</summary>
    public static bool IsDuplicate(this RememberResponse response) =>
        response.FactsWritten == 0;

    /// <summary>Return the per-statement results that did not succeed.</summary>
    public static IEnumerable<BatchRememberResult> Failures(this RememberBatchResponse response) =>
        (response.Results ?? []).Where(r => !r.Ok);
}
