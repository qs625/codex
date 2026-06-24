use std::path::Path;

use codex_protocol::request_permissions::PermissionGrantScope;
use codex_protocol::request_permissions::RequestPermissionProfile;
use codex_protocol::request_permissions::RequestPermissionsResponse;

use crate::policy_transforms::intersect_permission_profiles;

/// Normalizes a request-permissions response against the original request and cwd.
///
/// Session-scope strict auto review is intentionally downgraded to an empty turn
/// response because strict review is a turn-local safety mode. Non-empty granted
/// permissions are intersected with the original request before callers record
/// them as turn or session grants.
pub fn normalize_request_permissions_response(
    requested_permissions: RequestPermissionProfile,
    response: RequestPermissionsResponse,
    cwd: &Path,
) -> RequestPermissionsResponse {
    if response.strict_auto_review && matches!(response.scope, PermissionGrantScope::Session) {
        return RequestPermissionsResponse {
            permissions: RequestPermissionProfile::default(),
            scope: PermissionGrantScope::Turn,
            strict_auto_review: false,
        };
    }

    if response.permissions.is_empty() {
        return response;
    }

    RequestPermissionsResponse {
        permissions: intersect_permission_profiles(
            requested_permissions.into(),
            response.permissions.into(),
            cwd,
        )
        .into(),
        scope: response.scope,
        strict_auto_review: response.strict_auto_review,
    }
}

#[cfg(test)]
mod tests {
    use codex_protocol::models::NetworkPermissions;
    use codex_protocol::request_permissions::PermissionGrantScope;
    use codex_protocol::request_permissions::RequestPermissionProfile;
    use codex_protocol::request_permissions::RequestPermissionsResponse;
    use pretty_assertions::assert_eq;

    use super::normalize_request_permissions_response;

    fn network_request_permissions() -> RequestPermissionProfile {
        RequestPermissionProfile {
            network: Some(NetworkPermissions {
                enabled: Some(true),
            }),
            ..RequestPermissionProfile::default()
        }
    }

    #[test]
    fn strict_auto_review_session_scope_grants_no_permissions() {
        let requested_permissions = network_request_permissions();

        let response = normalize_request_permissions_response(
            requested_permissions.clone(),
            RequestPermissionsResponse {
                permissions: requested_permissions,
                scope: PermissionGrantScope::Session,
                strict_auto_review: true,
            },
            std::path::Path::new("/tmp"),
        );

        assert_eq!(
            response,
            RequestPermissionsResponse {
                permissions: RequestPermissionProfile::default(),
                scope: PermissionGrantScope::Turn,
                strict_auto_review: false,
            }
        );
    }
}
