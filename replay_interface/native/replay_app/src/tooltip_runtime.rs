
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Xui05ProductRouteStatus {
    pub packet: &'static str,
    pub available: bool,
    pub supported_routes: &'static [&'static str],
    pub feature_local_refusals: &'static [&'static str],
}

pub fn xui05_product_route_status() -> Xui05ProductRouteStatus {
    Xui05ProductRouteStatus {
        packet: "XUI05",
        available: true,
        supported_routes: &[
            "authenticated XUI02/XUI04 hovered authored node",
            "unscoped pdx_tooltip ordinary disabled and delayed declarations",
            "viewer-clock dwell hide and country-or-seek reset",
            "effective admitted localisation font chrome and clamp assets",
        ],
        feature_local_refusals: &[
            "custom provider without a packet-owned exact semantic payload",
            "scoped or runtime-substituted localisation without evaluation context",
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_status_names_supported_and_refused_families() {
        let status = xui05_product_route_status();
        assert_eq!(status.packet, "XUI05");
        assert!(status.available);
        assert_eq!(status.supported_routes.len(), 4);
        assert!(status
            .supported_routes
            .iter()
            .any(|contract| contract.contains("XUI02")));
        assert!(status
            .supported_routes
            .iter()
            .any(|contract| contract.contains("viewer-clock")));
        assert_eq!(status.feature_local_refusals.len(), 2);
        assert!(status
            .feature_local_refusals
            .iter()
            .any(|contract| contract.contains("custom provider")));
        assert!(status
            .feature_local_refusals
            .iter()
            .any(|contract| contract.contains("scoped")));
    }
}
