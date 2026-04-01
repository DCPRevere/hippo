namespace Hippo.Sdk;

/// <summary>
/// Convenience extension methods for Hippo response models.
/// </summary>
public static class ResponseExtensions
{
    /// <summary>Find a node by name (case-insensitive).</summary>
    public static ContextNode? FindNode(this ContextResponse response, string name) =>
        response.Nodes?.FirstOrDefault(n =>
            string.Equals(n.Name, name, StringComparison.OrdinalIgnoreCase));

    /// <summary>Return edges where the entity appears as source or target.</summary>
    public static IEnumerable<ContextEdge> FactsAbout(this ContextResponse response, string entityName) =>
        (response.Edges ?? []).Where(e =>
            string.Equals(e.Source, entityName, StringComparison.OrdinalIgnoreCase) ||
            string.Equals(e.Target, entityName, StringComparison.OrdinalIgnoreCase));

    /// <summary>True when nothing new was written (the statement was already known).</summary>
    public static bool IsDuplicate(this RememberResponse response) =>
        response.FactsWritten == 0;

    /// <summary>
    /// Return results that indicate failure (no entities created, resolved, or facts written).
    /// </summary>
    public static IEnumerable<RememberResponse> Failures(this RememberBatchResponse response) =>
        (response.Results ?? []).Where(r =>
            r.EntitiesCreated == 0 && r.EntitiesResolved == 0 && r.FactsWritten == 0);
}
