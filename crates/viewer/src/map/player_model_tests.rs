use super::scene_failure_tests::Fixture;
use crate::controls::flight::{Airframe, EXPLODE, Flight};
use std::io;
use viewer_runtime::errors::invalid;
use viewer_gpu::camera::Pose;
use viewer_gpu::province_map::HeightField;
use viewer_world::scene::{PlayerModel, Request};

#[test]
#[ignore = "verified replay/content and GPU; requires BATTLEPLAN_SCENE_FAILURE_INPUT"]
fn the_flying_aircraft_draws_whatever_the_map_cutoffs_and_layer_switches() -> io::Result<()> {
    let fixture = Fixture::load("player")?;
    let mut scene = fixture.scene()?;
    let document = &fixture.document;
    let (device, queue) = (&fixture.state.device, &fixture.state.queue);
    let aircraft = document
        .aircraft
        .roster()
        .descriptors
        .entities
        .keys()
        .find_map(|name| document.aircraft.player(name).ok())
        .ok_or_else(|| invalid("the content registers no entity that can be flown"))?;
    scene.set_graphics(dashboard_core::protocol::MapGraphics {
        samples: 1,
        units: false,
        ..Default::default()
    });
    let base = fixture.request([4, 4]).camera;
    assert_eq!(base.pose, Pose::Map);
    let defines = document.map_defines;
    let [x, _, z] = fixture.request([4, 4]).camera.eye();
    let field = HeightField {
        width: document.map.width,
        height: document.map.height,
        values: document.assets.heightmap_texels(),
    };
    let floor = |x: f32, z: f32| field.surface_height(x, z);
    let airframe = {
        let library = document.aircraft.library();
        Airframe {
            size: library.meshes[aircraft.slot].radius,
            explodes: library.has_state(aircraft.slot, EXPLODE),
            ground: document.aircraft.ground_contact().map_err(invalid)?,
        }
    };
    let mut flight = Flight::new([x, defines.map_objects_cutoff + 50.0, z], 1.0, 0.0, 0.0);
    flight.update(
        &crate::controls::Frame::default(),
        &base,
        &floor,
        &airframe,
        None,
        &mut crate::controls::flight::Footprints::default(),
    );
    let sight = flight.sight(base, &floor, &airframe);
    let instance = sight.aircraft.expect("the chase view draws the aircraft");
    assert!(!viewer_aircraft::aircraft::draws(&sight.camera));
    let request = Request {
        camera: sight.camera,
        animation_time: 1.0,
        postfx: false,
        ..fixture.request([320, 180])
    };
    let frame = |scene: &mut viewer_world::scene::Scene, player: bool| {
        let library = document.aircraft.library();
        scene.set_player(
            device,
            queue,
            player.then_some(PlayerModel {
                library: &library,
                label: crate::app::fly::LABEL,
                mesh: aircraft.slot,
                instance,
            }),
        )?;
        drop(library);
        scene.render(device, queue, request)?;
        scene.render(device, queue, request)?;
        io::Result::Ok(())
    };
    frame(&mut scene, false)?;
    assert_eq!(scene.player_drawn(), 0);
    let without = fixture.pixels(&scene, "without")?;
    frame(&mut scene, true)?;
    assert!(scene.player_drawn() >= 1, "{}", scene.player_drawn());
    assert_eq!(scene.aircraft_drawn(), 0);
    let with = fixture.pixels(&scene, "with")?;
    let changed = without
        .chunks_exact(4)
        .zip(with.chunks_exact(4))
        .filter(|(a, b)| a != b)
        .count();
    assert!(changed > 50, "{changed} pixels changed");
    frame(&mut scene, false)?;
    assert_eq!(fixture.pixels(&scene, "removed")?, without);
    Ok(())
}
