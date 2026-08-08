//! Model-picker list merging.

/// Merge local and cloud model entries `(id, label)` into picker order:
/// local entries first, and the FIRST entry for an id wins — a cloud
/// entry must never replace the local one that shadows it.
pub fn merge(local: &[(String, String)], cloud: &[(String, String)]) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for e in local.iter().chain(cloud.iter()) {
        if let Some(existing) = out.iter_mut().find(|x| x.0 == e.0) {
            *existing = e.clone();
        } else {
            out.push(e.clone());
        }
    }
    out
}
