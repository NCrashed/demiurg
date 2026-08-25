//! End-to-end manifest → document tests: what a Blender exporter writes must
//! come back out of a `.demiurg` / `.rkc` as the rig it described.

use std::path::Path;

use demiurg_convert::{Error, Output, convert};
use demiurg_core::project::{self, Loaded};

/// A two-bone rig with one clip: a `torso` root and a `arm` child that rotates
/// a quarter turn about `+Z` halfway through a 1-second loop.
fn two_bone_rig() -> String {
    // `r##`, because the manifest itself contains a `"#` (the `#a08040`).
    r##"{
      "format": "demiurg-rig",
      "version": 1,
      "name": "hero",
      "bones": [
        {
          "name": "torso",
          "mesh": { "dims": [3, 3, 6], "pivot": [1.5, 1.5, 0.0],
                    "voxels": [[1, 1, 1, "c0c0c0"], [1, 1, 2, "#a08040"]] }
        },
        {
          "name": "arm",
          "parent": "torso",
          "joint": [1.5, 0.0, 4.0],
          "mesh": { "dims": [2, 2, 5], "pivot": [1.0, 1.0, 0.0],
                    "voxels": [[0, 0, 0, "ff8800"]] }
        }
      ],
      "clips": [
        {
          "name": "wave",
          "loop": true,
          "length_ms": 1000,
          "keys": [
            { "t": 0,   "pose": { "arm": {} } },
            { "t": 500, "pose": { "arm": { "r": [0.0, 0.0, 0.70710678, 0.70710678],
                                           "t": [0.0, 0.0, 1.0] } } }
          ]
        }
      ]
    }"##
    .to_string()
}

fn load_rig(json: &str) -> demiurg_core::Rig {
    let out = convert(json.as_bytes(), Path::new("."), Output::Demiurg).expect("converts");
    match project::from_bytes(&out.bytes).expect("loads") {
        Loaded::Rig(r) => r,
        _ => panic!("expected a rig"),
    }
}

#[test]
fn rig_survives_the_demiurg_round_trip() {
    let rig = load_rig(&two_bone_rig());

    assert_eq!(rig.name, "hero");
    assert_eq!(rig.bones.len(), 2);
    assert_eq!(rig.bones[0].name, "torso");
    assert_eq!(rig.bones[1].name, "arm");
    // Parenting, and the joint as the parent-side anchor — stored negated,
    // because the solver subtracts it (see the solved-offset test above for
    // the invariant that actually matters).
    assert_eq!(rig.bones[0].hinge.parent, -1);
    assert_eq!(rig.bones[1].hinge.parent, 0);
    assert!((rig.bones[1].hinge.p[1].z + 4.0).abs() < 1e-6);
    // The child attaches by its own pivot, so the child-side anchor is zero.
    assert!(rig.bones[1].hinge.p[0].x.abs() < 1e-6);
    // Meshes, with the pivot the exporter asked for (what `.vox` can't carry).
    assert_eq!(rig.bones[0].model.dims(), (3, 3, 6));
    assert_eq!(rig.bones[0].model.occupied_count(), 2);
    assert_eq!(rig.bones[0].model.get(1, 1, 1), 0x80c0_c0c0);
    assert!((rig.bones[0].model.pivot[2] - 0.0).abs() < 1e-6);
    assert_eq!(rig.bones[1].model.get(0, 0, 0), 0x80ff_8800);
}

#[test]
fn a_joint_puts_the_child_where_the_manifest_says() {
    // The invariant an exporter relies on: `joint` is where the child's pivot
    // lands, measured from the parent's, in the parent's frame. The engine's
    // hinge anchor is the negation of that, so this is asserted through the
    // real solver — a sign error here reads perfectly in the struct and hangs
    // every limb off the opposite side of its parent on screen.
    let json = r#"{
      "format": "demiurg-rig", "version": 1,
      "bones": [
        { "name": "root", "mesh": { "dims": [1, 1, 1], "pivot": [0.0, 0.0, 0.0],
                                    "voxels": [[0, 0, 0, "ffffff"]] } },
        { "name": "child", "parent": "root", "joint": [2.0, 3.0, -4.0],
          "mesh": { "dims": [1, 1, 1], "pivot": [0.0, 0.0, 0.0],
                    "voxels": [[0, 0, 0, "ffffff"]] } }
      ]
    }"#;
    let rig = load_rig(json);
    let mut sprite = rig.to_character().to_kfa_sprite(None);
    roxlap_core::kfa_draw::solve_kfa_limbs(&mut sprite);

    let root = sprite.limbs[0].p;
    let child = sprite.limbs[1].p;
    let offset = [child[0] - root[0], child[1] - root[1], child[2] - root[2]];
    for (got, want) in offset.iter().zip([2.0_f32, 3.0, -4.0]) {
        assert!(
            (got - want).abs() < 1e-4,
            "solved offset {offset:?} should equal the manifest's joint"
        );
    }
}

#[test]
fn a_child_bone_comes_out_animatable() {
    let rig = load_rig(&two_bone_rig());
    // `htype != 0` would pin the bone to rest, `vmin == vmax` would lock it out
    // of the editor's posing — both silent failures, so assert them explicitly.
    assert_eq!(rig.bones[1].hinge.htype, 0);
    assert!(rig.bones[1].hinge.vmin < rig.bones[1].hinge.vmax);
    assert!(rig.is_poseable(1));
}

#[test]
fn clip_keys_land_at_their_times_with_the_pose_intact() {
    let rig = load_rig(&two_bone_rig());
    assert_eq!(rig.clips.len(), 1);
    assert_eq!(rig.clips[0].name, "wave");

    let keys = rig.clip_keyframes(0);
    assert_eq!(
        keys.len(),
        2,
        "the seeded rest key was overwritten, not kept"
    );
    assert_eq!(keys[0].tim, 0);
    assert_eq!(keys[1].tim, 500);
    // The quarter turn about +Z, scalar-last, survives verbatim.
    let arm = keys[1].xforms[1];
    let half_sqrt2 = std::f32::consts::FRAC_1_SQRT_2;
    assert!((arm.r.z - half_sqrt2).abs() < 1e-6);
    assert!((arm.r.w - half_sqrt2).abs() < 1e-6);
    assert!((arm.t[2] - 1.0).abs() < 1e-6);
    // An omitted bone is posed to rest — whole-skeleton keys, not deltas.
    assert_eq!(keys[1].xforms[0], demiurg_core::KeyXform::IDENTITY);
    // The loop marker sits where `length_ms` put it, so the closing segment
    // (back to key 0) is the remaining 500 ms.
    assert_eq!(rig.clip_loop_tim(0), 1000);
    assert!(rig.clip_loops(0));
}

#[test]
fn a_one_shot_clip_holds_its_last_pose() {
    let json = r#"{
      "format": "demiurg-rig", "version": 1,
      "bones": [ { "name": "a" }, { "name": "b", "parent": "a" } ],
      "clips": [ { "name": "hit", "loop": false,
                   "keys": [ { "t": 0, "pose": {} }, { "t": 200, "pose": {} } ] } ]
    }"#;
    let rig = load_rig(json);
    assert!(!rig.clip_loops(0));
    assert_eq!(rig.clip_keyframes(0).len(), 2);
}

#[test]
fn the_seeded_rest_key_is_dropped_when_the_clip_starts_later() {
    // `Rig::add_clip` seeds a key at t=0 so the editor timeline is never
    // empty; a manifest whose first key is later must not inherit it.
    let json = r#"{
      "format": "demiurg-rig", "version": 1,
      "bones": [ { "name": "a" }, { "name": "b", "parent": "a" } ],
      "clips": [ { "name": "late", "keys": [ { "t": 100, "pose": {} } ] } ]
    }"#;
    let rig = load_rig(json);
    let keys = rig.clip_keyframes(0);
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0].tim, 100);
}

#[test]
fn rkc_output_is_a_loadable_character() {
    let out = convert(two_bone_rig().as_bytes(), Path::new("."), Output::Rkc).expect("converts");
    let rig = demiurg_core::Rig::from_rkc_bytes(&out.bytes).expect("parses as .rkc");
    assert_eq!(rig.bones.len(), 2);
    assert_eq!(rig.clip_keyframes(0).len(), 2);
    assert_eq!(out.stats.bones, 2);
    assert_eq!(out.stats.clips, 1);
    assert_eq!(out.stats.keys, 2);
    assert_eq!(out.stats.voxels, 3);
}

#[test]
fn a_model_manifest_writes_a_bare_model_with_its_pivot() {
    let json = r#"{
      "format": "demiurg-model", "version": 1,
      "mesh": { "dims": [4, 4, 4], "pivot": [2.0, 2.0, 0.5],
                "voxels": [[0, 1, 2, "112233"]] }
    }"#;
    let out = convert(json.as_bytes(), Path::new("."), Output::Demiurg).expect("converts");
    let Loaded::Model(m) = project::from_bytes(&out.bytes).expect("loads") else {
        panic!("expected a model");
    };
    assert_eq!(m.dims(), (4, 4, 4));
    assert_eq!(m.get(0, 1, 2), 0x8011_2233);
    assert!((m.pivot[2] - 0.5).abs() < 1e-6);
}

#[test]
fn a_model_cannot_be_written_as_rkc() {
    let json = r#"{ "format": "demiurg-model", "version": 1,
                    "mesh": { "dims": [1, 1, 1] } }"#;
    assert!(matches!(
        convert(json.as_bytes(), Path::new("."), Output::Rkc),
        Err(Error::ModelToRkc)
    ));
}

#[test]
fn a_vox_file_mesh_is_imported_beside_the_manifest() {
    let dir = std::env::temp_dir().join("demiurg-convert-vox-test");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let mut model = demiurg_core::VoxelModel::new(2, 2, 2);
    model.set(1, 0, 0, 0x8012_3456);
    std::fs::write(dir.join("arm.vox"), demiurg_core::vox::serialize(&model)).expect("write .vox");

    let json = r#"{
      "format": "demiurg-rig", "version": 1,
      "bones": [ { "name": "a", "mesh": { "vox_file": "arm.vox", "pivot": [1.0, 1.0, 0.0] } } ]
    }"#;
    let out = convert(json.as_bytes(), &dir, Output::Demiurg).expect("converts");
    let Loaded::Rig(rig) = project::from_bytes(&out.bytes).expect("loads") else {
        panic!("expected a rig");
    };
    assert_eq!(rig.bones[0].model.dims(), (2, 2, 2));
    assert_eq!(rig.bones[0].model.occupied_count(), 1);
    assert!((rig.bones[0].model.pivot[0] - 1.0).abs() < 1e-6);

    std::fs::remove_dir_all(&dir).ok();
}

/// The error message for `json` must mention `needle` — these are read by
/// someone debugging a Python exporter, so they have to name the culprit.
fn rejects(json: &str, needle: &str) {
    let err = convert(json.as_bytes(), Path::new("."), Output::Demiurg)
        .err()
        .unwrap_or_else(|| panic!("expected a failure mentioning {needle:?}"))
        .to_string();
    assert!(err.contains(needle), "{err:?} should mention {needle:?}");
}

#[test]
fn animating_a_root_bone_is_rejected() {
    // The solver hands a root bone the sprite's own basis and ignores its
    // keyframe, so this would silently export a limp rig.
    rejects(
        r#"{
          "format": "demiurg-rig", "version": 1,
          "bones": [ { "name": "torso" } ],
          "clips": [ { "name": "spin", "keys": [
            { "t": 0, "pose": { "torso": { "r": [0.0, 0.0, 0.7071, 0.7071] } } } ] } ]
        }"#,
        "torso",
    );
}

#[test]
fn a_root_bone_may_still_carry_an_identity_key() {
    // Baked exports write every bone every key; a rest-pose root is fine.
    let json = r#"{
      "format": "demiurg-rig", "version": 1,
      "bones": [ { "name": "torso" }, { "name": "arm", "parent": "torso" } ],
      "clips": [ { "name": "idle", "keys": [
        { "t": 0, "pose": { "torso": { "r": [0.0, 0.0, 0.0, 1.0] }, "arm": {} } } ] } ]
    }"#;
    assert_eq!(load_rig(json).clip_keyframes(0).len(), 1);
}

#[test]
fn structural_mistakes_are_rejected_by_name() {
    rejects(
        r#"{ "format": "demiurg-rig", "version": 1,
             "bones": [ { "name": "a" }, { "name": "a" } ] }"#,
        "\"a\"",
    );
    rejects(
        r#"{ "format": "demiurg-rig", "version": 1,
             "bones": [ { "name": "a", "parent": "nope" } ] }"#,
        "nope",
    );
    rejects(
        r#"{ "format": "demiurg-rig", "version": 1,
             "bones": [ { "name": "a", "parent": "b" }, { "name": "b", "parent": "a" } ] }"#,
        "cycle",
    );
    rejects(
        r#"{ "format": "demiurg-rig", "version": 1,
             "bones": [ { "name": "a" }, { "name": "b", "parent": "a" } ],
             "clips": [ { "name": "c", "keys": [ { "t": 0, "pose": { "ghost": {} } } ] } ] }"#,
        "ghost",
    );
    rejects(
        r#"{ "format": "demiurg-rig", "version": 1,
             "bones": [ { "name": "a", "axis": [0.0, 0.0, 0.0] } ] }"#,
        "axis",
    );
    rejects(
        r#"{ "format": "demiurg-rig", "version": 1,
             "bones": [ { "name": "a", "mesh": { "dims": [2, 2, 2],
                          "voxels": [[5, 0, 0, "ffffff"]] } } ] }"#,
        "outside dims",
    );
    rejects(
        r#"{ "format": "demiurg-rig", "version": 1,
             "bones": [ { "name": "a", "mesh": { "dims": [2, 2, 2],
                          "voxels": [[0, 0, 0, "nothex"]] } } ] }"#,
        "hex colour",
    );
    rejects(
        r#"{ "format": "demiurg-rig", "version": 1,
             "bones": [ { "name": "a", "mesh": { "dims": [1, 1, 1], "vox_file": "x.vox" } } ] }"#,
        "not both or neither",
    );
    rejects(
        r#"{ "format": "demiurg-rig", "version": 1, "bones": [] }"#,
        "at least one bone",
    );
    rejects(r#"{ "format": "demiurg-rig", "version": 9 }"#, "version 9");
    rejects(
        r#"{ "format": "voxelity", "version": 1 }"#,
        "unknown format",
    );
}

#[test]
fn a_misspelled_field_is_rejected_rather_than_ignored() {
    // A silently-ignored `"pivots"` would export a rig that rotates about the
    // wrong point, which is far harder to debug than a parse error.
    rejects(
        r#"{ "format": "demiurg-rig", "version": 1,
             "bones": [ { "name": "a", "mesh": { "dims": [1, 1, 1], "pivots": [0, 0, 0] } } ] }"#,
        "pivots",
    );
}
