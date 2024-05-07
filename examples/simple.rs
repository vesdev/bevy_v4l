use argh::FromArgs;
use bevy::prelude::*;
use bevy_v4l::{Input, V4lPlugin};

#[derive(FromArgs)]
/// Simple input capture
struct Args {
    /// input device id
    #[argh(positional)]
    device: usize,
}

fn main() {
    App::new()
        .add_plugins((DefaultPlugins, V4lPlugin))
        .add_systems(Startup, setup)
        .run();
}

fn setup(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    let args: Args = argh::from_env();
    commands.spawn(Camera2dBundle::default());
    let device = Input::new(args.device, &mut images).unwrap();
    commands.spawn((
        SpriteBundle {
            texture: device.image().clone(),
            ..default()
        },
        device,
    ));
}
