use bevy::prelude::*;
use bevy_v4l::{Decoder, V4lDevice, V4lPlugin};
fn main() {
    App::new()
        .add_plugins((DefaultPlugins, V4lPlugin))
        .add_systems(Startup, setup)
        .run();
}

fn setup(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    commands.spawn(Camera2dBundle::default());
    let device = V4lDevice::new(0, &mut images).unwrap();
    commands.spawn((
        SpriteBundle {
            texture: device.image().clone(),
            ..default()
        },
        device,
        Decoder::default(),
    ));
}
