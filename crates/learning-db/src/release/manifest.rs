//! Complete fixed release closure and canonical manifest; display filtering is never used.
use crate::{composition::closure, overlay::model, reading::selection, references};
use learning_core::*;
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;
pub(super) struct Manifest {
    pub roots: Vec<CompositionRef>,
    pub compositions: BTreeMap<CompositionRef, Uuid>,
    pub objects: Vec<(String, Uuid, Uuid)>,
    pub readings: Vec<model::Layer>,
    pub spaces: Vec<(Uuid, bool)>,
}
impl Manifest {
    pub fn digest(&self) -> String {
        let objects:Vec<_>=self.objects.iter().map(|(kind,object_id,revision_id)|serde_json::json!({"kind":kind,"object_id":object_id,"revision_id":revision_id})).collect();
        let compositions: Vec<_> = self.compositions.keys().collect();
        let readings: Vec<_> = self.readings.iter().map(|r| &r.view).collect();
        hex_digest(canonical_json(&serde_json::json!({"domain":"release-manifest-v2","roots":self.roots,"compositions":compositions,"objects":objects,"readings":readings})).as_bytes())
    }
}
pub(super) async fn collect(
    tx: &mut model::Tx<'_>,
    actor: Principal,
    roots: &[CompositionRef],
    readings: &[ReadingRef],
) -> Result<Manifest, ContentError> {
    let mut composition_roots: BTreeSet<_> = roots.iter().cloned().collect();
    let mut exact_roots = BTreeSet::new();
    let mut layers = vec![];
    let mut spaces = vec![];
    for view in readings {
        let layer = model::load_view(tx, actor, view.clone()).await?;
        spaces.push((layer.space, false));
        composition_roots.insert(layer.data.base.clone());
        for group in &layer.data.groups {
            composition_roots.insert(group.location.anchor().base.clone());
            exact_roots.extend(
                group
                    .placements
                    .iter()
                    .map(|p| ExactRef::Block(p.block.clone())),
            );
        }
        let (_, choices) = selection::choices(tx, view).await?;
        exact_roots.extend(choices.references());
        layers.push(layer);
    }
    layers.sort_by_key(|l| l.view.clone());
    let composition_roots: Vec<_> = composition_roots.into_iter().collect();
    // B1 composition limits still apply. The following merged exact closure also
    // counts every root body and all selected evidence once under B3 limits.
    let compositions = closure::load(tx, actor, &composition_roots, None).await?;
    spaces.extend(compositions.spaces());
    exact_roots.extend(compositions.blocks.keys().cloned().map(ExactRef::Block));
    let exact = references::load(tx, actor, &exact_roots.into_iter().collect::<Vec<_>>()).await?;
    spaces.extend(exact.spaces());
    let mut objects: Vec<_> = exact
        .objects
        .keys()
        .map(|r| {
            let (k, o, v) = references::key(r);
            (k.to_owned(), o, v)
        })
        .collect();
    objects.sort();
    let mut roots = roots.to_vec();
    roots.sort();
    Ok(Manifest {
        roots,
        compositions: compositions
            .compositions
            .into_iter()
            .map(|(r, (s, _))| (r, s))
            .collect(),
        objects,
        readings: layers,
        spaces,
    })
}
