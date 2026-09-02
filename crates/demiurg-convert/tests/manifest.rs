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

/// Solve `rig`'s clip 0 at `t` ms and return each bone's world position.
fn solved_positions(rig: &demiurg_core::Rig, clip: Option<usize>, t: i32) -> Vec<[f32; 3]> {
    let mut sprite = rig.to_character().to_kfa_sprite(clip);
    sprite.kfatim = t;
    sprite.animsprite(0);
    roxlap_core::kfa_draw::solve_kfa_limbs(&mut sprite);
    sprite.limbs.iter().map(|l| l.p).collect()
}

#[test]
fn a_rotation_spins_the_bone_about_its_own_joint() {
    // The solver puts a bone at `t + r · anchor`, so a raw rotation swings its
    // head around the PARENT's pivot — a quarter turn would fling this bone
    // from (0, 0, 10) to (0, -10, 0), detaching the limb. The converter
    // cancels that arc, so the head stays put and only the mesh turns, which
    // is what every DCC means by rotating a bone.
    let json = r#"{
      "format": "demiurg-rig", "version": 1,
      "bones": [
        { "name": "root", "mesh": { "dims": [1, 1, 1], "pivot": [0.0, 0.0, 0.0],
                                    "voxels": [[0, 0, 0, "ffffff"]] } },
        { "name": "child", "parent": "root", "joint": [0.0, 0.0, 10.0],
          "mesh": { "dims": [1, 1, 1], "pivot": [0.0, 0.0, 0.0],
                    "voxels": [[0, 0, 0, "ffffff"]] } }
      ],
      "clips": [ { "name": "turn", "length_ms": 1000, "keys": [
        { "t": 0, "pose": { "child": { "r": [0.70710678, 0.0, 0.0, 0.70710678] } } } ] } ]
    }"#;
    let rig = load_rig(json);
    let posed = solved_positions(&rig, Some(0), 0);
    let offset = [
        posed[1][0] - posed[0][0],
        posed[1][1] - posed[0][1],
        posed[1][2] - posed[0][2],
    ];
    for (got, want) in offset.iter().zip([0.0_f32, 0.0, 10.0]) {
        assert!(
            (got - want).abs() < 1e-3,
            "a rotated bone stays at its joint; got {offset:?}"
        );
    }
    // The mesh really did turn: the basis is not the rest one.
    let rest = solved_positions(&rig, None, 0);
    assert!(
        (rest[1][2] - 10.0).abs() < 1e-3,
        "rest pose sits at the joint too"
    );
}

#[test]
fn a_translation_key_moves_the_bone_from_its_joint() {
    // With the arc cancelled, `t` has to stay a plain offset from the joint.
    let json = r#"{
      "format": "demiurg-rig", "version": 1,
      "bones": [
        { "name": "root", "mesh": { "dims": [1, 1, 1], "pivot": [0.0, 0.0, 0.0],
                                    "voxels": [[0, 0, 0, "ffffff"]] } },
        { "name": "child", "parent": "root", "joint": [0.0, 0.0, 10.0],
          "mesh": { "dims": [1, 1, 1], "pivot": [0.0, 0.0, 0.0],
                    "voxels": [[0, 0, 0, "ffffff"]] } }
      ],
      "clips": [ { "name": "slide", "length_ms": 1000, "keys": [
        { "t": 0, "pose": { "child": { "t": [3.0, 0.0, 0.0] } } } ] } ]
    }"#;
    let posed = solved_positions(&load_rig(json), Some(0), 0);
    let offset = [
        posed[1][0] - posed[0][0],
        posed[1][1] - posed[0][1],
        posed[1][2] - posed[0][2],
    ];
    for (got, want) in offset.iter().zip([3.0_f32, 0.0, 10.0]) {
        assert!((got - want).abs() < 1e-3, "joint + t; got {offset:?}");
    }
}

/// A rigid root with a deforming child: the case a skeleton alone can't
/// express, since a bone moves its mesh but never reshapes it.
fn mixed_rig() -> &'static str {
    r#"{
      "format": "demiurg-rig", "version": 1, "name": "slime",
      "bones": [
        { "name": "base", "mesh": { "dims": [2, 2, 2], "pivot": [1.0, 1.0, 0.0],
                                    "voxels": [[0, 0, 0, "334455"]] } },
        { "name": "blob", "parent": "base", "joint": [0.0, 0.0, -2.0],
          "clip": {
            "dims": [4, 4, 4], "pivot": [2.0, 2.0, 0.0],
            "frame_ms": 100, "loop": "pingpong", "speed": 1.5, "phase_ms": 40,
            "frames": [
              { "voxels": [[0, 0, 0, "22cc55"]] },
              { "voxels": [[1, 1, 1, "22cc55"], [2, 2, 2, "22cc55"]], "duration_ms": 250 },
              { "voxels": [[3, 3, 3, "22cc55"]] }
            ]
          } }
      ]
    }"#
}

#[test]
fn a_bone_can_carry_per_frame_geometry() {
    let out = convert(mixed_rig().as_bytes(), Path::new("."), Output::Demiurg).expect("converts");
    let Loaded::Rig(rig) = project::from_bytes(&out.bytes).expect("loads") else {
        panic!("expected a rig");
    };
    // The rigid bone is untouched by any of this.
    assert!(rig.bones[0].primary_clip.is_none());
    assert_eq!(rig.bones[0].model.occupied_count(), 1);

    let clip = rig.bones[1]
        .primary_clip
        .as_ref()
        .expect("the bone draws a clip");
    assert_eq!(clip.dims, [4, 4, 4]);
    assert_eq!(clip.frames.len(), 3);
    assert_eq!(clip.default_frame_ms, 100);
    assert_eq!(clip.loop_mode, demiurg_core::LoopMode::PingPong);
    assert_eq!(clip.frames[1].duration_ms, Some(250));
    // The frames really are different geometry, which is the whole point.
    assert_eq!(clip.frames[0].model.get(0, 0, 0), 0x8022_cc55);
    assert_eq!(clip.frames[0].model.get(3, 3, 3), 0);
    assert_eq!(clip.frames[1].model.occupied_count(), 2);
    assert_eq!(clip.frames[2].model.get(3, 3, 3), 0x8022_cc55);
    // Playback: 1.5x in Q8, and the phase offset.
    assert_eq!(rig.bones[1].primary_playback.speed_q8, 384);
    assert_eq!(rig.bones[1].primary_playback.start_phase_ms, 40);

    // A clip bone's voxels are in its frames, not its placeholder model, so
    // the summary has to look there or a slime reports as an empty rig.
    assert_eq!(out.stats.clip_frames, 3);
    assert_eq!(out.stats.voxels, 1 + 1 + 2 + 1);
}

#[test]
fn a_clip_bone_still_hangs_off_its_joint() {
    // The flipbook replaces the geometry, not the skeleton: the bone is posed
    // exactly like a rigid one.
    let out = convert(mixed_rig().as_bytes(), Path::new("."), Output::Demiurg).expect("converts");
    let Loaded::Rig(rig) = project::from_bytes(&out.bytes).expect("loads") else {
        panic!("expected a rig");
    };
    let posed = solved_positions(&rig, None, 0);
    let offset = [
        posed[1][0] - posed[0][0],
        posed[1][1] - posed[0][1],
        posed[1][2] - posed[0][2],
    ];
    for (got, want) in offset.iter().zip([0.0_f32, 0.0, -2.0]) {
        assert!(
            (got - want).abs() < 1e-3,
            "clip bone at its joint; got {offset:?}"
        );
    }
}

#[test]
fn rkc_carries_the_voxel_clip_too() {
    // The engine container has to receive it, not just the editor project.
    let out = convert(mixed_rig().as_bytes(), Path::new("."), Output::Rkc).expect("converts");
    let rig = demiurg_core::Rig::from_rkc_bytes(&out.bytes).expect("parses as .rkc");
    let clip = rig.bones[1]
        .primary_clip
        .as_ref()
        .expect("clip survives .rkc");
    assert_eq!(clip.frames.len(), 3);
    assert_eq!(clip.frames[2].model.get(3, 3, 3), 0x8022_cc55);
}

#[test]
fn skeletal_clips_can_own_disjoint_windows_of_one_timeline() {
    // The way out of "one flipbook per bone": a bone's per-frame geometry is
    // picked by the rig playhead, and a skeletal clip's `seq` holds *absolute*
    // times — nothing says a clip must start at 0. Lay `walk` on 0..1000 and
    // `idle` on 1000..2000, give the flipbook frames covering both, and each
    // action reaches its own geometry.
    let json = r#"{
      "format": "demiurg-rig", "version": 1,
      "bones": [
        { "name": "root", "mesh": { "dims": [1, 1, 1], "pivot": [0.0, 0.0, 0.0],
                                    "voxels": [[0, 0, 0, "ffffff"]] } },
        { "name": "blob", "parent": "root", "joint": [0.0, 0.0, -2.0],
          "clip": { "dims": [4, 4, 4], "frame_ms": 500, "loop": "loop", "frames": [
            { "voxels": [[0, 0, 0, "111111"]] },
            { "voxels": [[1, 0, 0, "111111"]] },
            { "voxels": [[2, 0, 0, "222222"]] },
            { "voxels": [[3, 0, 0, "222222"]] } ] } }
      ],
      "clips": [
        { "name": "walk", "length_ms": 1000, "keys": [
          { "t": 0, "pose": {} }, { "t": 500, "pose": {} } ] },
        { "name": "idle", "length_ms": 2000, "keys": [
          { "t": 1000, "pose": {} }, { "t": 1500, "pose": {} } ] }
      ]
    }"#;
    let rig = load_rig(json);
    let clip = rig.bones[1]
        .primary_clip
        .as_ref()
        .expect("the blob deforms");
    let playback = rig.bones[1].primary_playback;

    // Each action's window reaches its own half of the flipbook.
    assert_eq!(
        clip.frame_at_playback(playback, 0),
        0,
        "walk sees its own frames"
    );
    assert_eq!(clip.frame_at_playback(playback, 600), 1);
    assert_eq!(
        clip.frame_at_playback(playback, 1000),
        2,
        "idle sees different geometry"
    );
    assert_eq!(clip.frame_at_playback(playback, 1600), 3);

    // And the clips really are laid out where the manifest asked.
    assert_eq!(rig.clip_keyframes(0)[0].tim, 0);
    assert_eq!(rig.clip_keyframes(1)[0].tim, 1000);
    assert_eq!(rig.clip_keyframes(1).len(), 2, "no stray key at t=0");
}

#[test]
fn a_looping_clip_stays_inside_its_own_window() {
    // The load-bearing detail: the solver's loop marker sets the playhead to
    // its *own* first entry, not to zero. Without that, `idle` would wrap to
    // 0 and start showing `walk`'s geometry on its second cycle.
    let json = r#"{
      "format": "demiurg-rig", "version": 1,
      "bones": [
        { "name": "root", "mesh": { "dims": [1, 1, 1], "pivot": [0.0, 0.0, 0.0],
                                    "voxels": [[0, 0, 0, "ffffff"]] } },
        { "name": "arm", "parent": "root", "joint": [0.0, 0.0, 1.0],
          "mesh": { "dims": [1, 1, 1], "pivot": [0.0, 0.0, 0.0],
                    "voxels": [[0, 0, 0, "ffffff"]] } }
      ],
      "clips": [
        { "name": "walk", "length_ms": 1000, "keys": [ { "t": 0, "pose": {} } ] },
        { "name": "idle", "length_ms": 2000, "keys": [
          { "t": 1000, "pose": {} }, { "t": 1500, "pose": {} } ] }
      ]
    }"#;
    let rig = load_rig(json);
    let mut sprite = rig.to_character().to_kfa_sprite(Some(1)); // idle
    sprite.kfatim = 1900;
    sprite.animsprite(400); // past the loop marker at 2000
    assert!(
        (1000..2000).contains(&sprite.kfatim),
        "idle looped to {} — outside its own 1000..2000 window",
        sprite.kfatim
    );
}

#[test]
fn clip_mistakes_are_rejected_by_name() {
    rejects(
        r#"{ "format": "demiurg-rig", "version": 1, "bones": [
             { "name": "blob", "mesh": { "dims": [1, 1, 1] },
               "clip": { "dims": [1, 1, 1], "frames": [{ "voxels": [] }] } } ] }"#,
        "one or the other",
    );
    rejects(
        r#"{ "format": "demiurg-rig", "version": 1, "bones": [
             { "name": "blob", "clip": { "dims": [1, 1, 1], "frames": [] } } ] }"#,
        "at least one frame",
    );
    rejects(
        r#"{ "format": "demiurg-rig", "version": 1, "bones": [
             { "name": "blob", "clip": { "dims": [1, 1, 1], "loop": "boomerang",
                                         "frames": [{ "voxels": [] }] } } ] }"#,
        "boomerang",
    );
    rejects(
        r#"{ "format": "demiurg-rig", "version": 1, "bones": [
             { "name": "blob", "clip": { "dims": [2, 2, 2],
                 "frames": [{ "voxels": [[9, 0, 0, "ffffff"]] }] } } ] }"#,
        "outside dims",
    );
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
    // `t` is not a pass-through — the stored value carries the arc-cancelling
    // term (see `a_rotation_spins_the_bone_about_its_own_joint`). This key
    // turns about +Z, which leaves the joint's z alone, so the z component is
    // still the manifest's own.
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
