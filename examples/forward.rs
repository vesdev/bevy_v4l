use argh::FromArgs;
use bevy::{prelude::*, window::ExitCondition};
use bevy_v4l::{Input, Output, V4lPlugin};

#[derive(FromArgs)]
/// Simple input capture
struct Args {
    /// input device id
    #[argh(positional)]
    input_device: usize,

    /// output device id
    #[argh(positional)]
    output_device: usize,
}

fn main() {
    App::new()
        .add_plugins((
            DefaultPlugins.build().set(WindowPlugin {
                primary_window: None,
                exit_condition: ExitCondition::DontExit,
                close_when_requested: false,
            }),
            V4lPlugin,
        ))
        .add_systems(Startup, setup)
        .run();
}

fn setup(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    let args: Args = argh::from_env();
    let mut input = Input::new(args.input_device, &mut images).unwrap();
    let image = input.clone_image(&mut images);

    let output = Output::new(args.output_device, input.image().clone(), input.format()).unwrap();

    commands.spawn((
        SpriteBundle {
            texture: input.image().clone(),
            ..default()
        },
        input,
    ));

    commands.spawn((
        Camera2dBundle {
            camera: Camera {
                target: image.clone().into(),
                ..default()
            },
            ..default()
        },
        output,
    ));
}
