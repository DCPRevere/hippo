package hippo

import "fmt"

// HippoError represents an HTTP error response from the Hippo API.
type HippoError struct {
	// StatusCode is the HTTP status code returned by the server.
	StatusCode int
	// Message is the human-readable error description.
	Message string
}

// Error implements the error interface.
func (e *HippoError) Error() string {
	return fmt.Sprintf("hippo: HTTP %d: %s", e.StatusCode, e.Message)
}

// IsAuthError reports whether the error is a 401 Unauthorized response.
func (e *HippoError) IsAuthError() bool {
	return e.StatusCode == 401
}

// IsForbidden reports whether the error is a 403 Forbidden response.
func (e *HippoError) IsForbidden() bool {
	return e.StatusCode == 403
}

// IsRateLimited reports whether the error is a 429 Too Many Requests response.
func (e *HippoError) IsRateLimited() bool {
	return e.StatusCode == 429
}
