use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use bevy::render::render_asset::RenderAssetUsages;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::tasks::{ComputeTaskPool, Task};
use bevy::utils::futures;
use ffimage::color::Rgb;
use ffimage::iter::{BytesExt, ColorConvertExt, PixelsExt};
use ffimage_yuv::yuv::Yuv;
use ffimage_yuv::yuv422::Yuv422;
use thiserror::Error;
use v4l::io::mmap::Stream;
use v4l::io::traits::CaptureStream;
use v4l::prelude::*;
use v4l::video::Capture;

type Result<T> = std::result::Result<T, Error>;

#[derive(Error, Debug)]
pub enum Error {
    #[error("v4l device unavailable")]
    Io(#[from] std::io::Error),
}

#[allow(dead_code)]
#[derive(Component)]
pub struct V4lDevice {
    id: usize,
    dev: v4l::Device,
    format: v4l::Format,
    stream: Arc<Mutex<Stream<'static>>>,
    image: Handle<Image>,
    buffer: Arc<Mutex<Vec<u8>>>,
}

impl V4lDevice {
    pub fn new(device_id: usize, images: &mut ResMut<Assets<Image>>) -> Result<Self> {
        let dev = v4l::Device::new(device_id)?;
        let format = dev.format()?;
        let stream = MmapStream::with_buffers(&dev, v4l::buffer::Type::VideoCapture, 4)?;

        let size = Extent3d {
            width: format.width,
            height: format.height,
            depth_or_array_layers: 1,
        };

        let buffer1 = vec![255_u8; (size.width * size.height * 4) as usize];
        let buffer2 = buffer1.clone();

        let image = images.add(Image::new(
            size,
            TextureDimension::D2,
            buffer1,
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::all(),
        ));

        Ok(Self {
            id: device_id,
            dev,
            format,
            stream: Arc::new(Mutex::new(stream)),
            image,
            buffer: Arc::new(Mutex::new(buffer2)),
        })
    }

    pub fn image(&self) -> &Handle<Image> {
        &self.image
    }
}

pub struct V4lPlugin;
impl Plugin for V4lPlugin {
    fn build(&self, app: &mut bevy::prelude::App) {
        app.add_systems(PreUpdate, decode)
            .add_systems(Update, present);
    }
}

#[derive(Component, Default)]
pub struct Decoder(Option<Task<()>>);

fn present(mut decoders: Query<(&mut Decoder, &V4lDevice)>, mut images: ResMut<Assets<Image>>) {
    for (mut task, device) in decoders.iter_mut() {
        let Some(mut task_status) = task.0.as_mut() else {
            continue;
        };

        if let Some(()) = futures::check_ready(&mut task_status) {
            let Some(image) = images.get_mut(device.image.clone()) else {
                continue;
            };
            let Ok(mut buffer) = device.buffer.lock() else {
                continue;
            };
            std::mem::swap(&mut image.data, &mut buffer);
            task.0 = None;
        }
    }
}

fn decode(mut devices: Query<(&mut V4lDevice, &mut Decoder)>, mut images: ResMut<Assets<Image>>) {
    for (device, mut decoder) in devices.iter_mut() {
        let Some(image) = images.get_mut(device.image.clone()) else {
            continue;
        };

        // task is unfinished
        if decoder.0.is_some() {
            continue;
        };

        let fourcc = device.format.fourcc.repr;
        let stream = device.stream.clone();
        let size = image.width() * image.height() * 4;
        let buffer = device.buffer.clone();
        let task = ComputeTaskPool::get()
            .spawn(async move { decode_to_rgba(stream, buffer, &fourcc, size as usize).await });

        decoder.0 = Some(task);
    }
}

async fn decode_to_rgba(
    stream: Arc<Mutex<Stream<'static>>>,
    buffer: Arc<Mutex<Vec<u8>>>,
    fourcc: &[u8; 4],
    size: usize,
) {
    let Ok(mut stream) = stream.lock() else {
        return;
    };

    let Ok(mut buffer) = buffer.lock() else {
        return;
    };

    let Ok((buf, _)) = stream.next() else {
        return;
    };

    // TODO: support other formats
    match fourcc {
        b"YUYV" => {
            let rgb = buf
                .iter()
                .copied()
                .pixels::<Yuv422<u8, 0, 2, 1, 3>>()
                .colorconvert::<[Yuv<u8>; 2]>()
                .flatten()
                .colorconvert::<Rgb<u8>>()
                .bytes()
                .enumerate();

            for (i, pixel) in rgb {
                let i = i * 4;

                if i >= size {
                    break;
                }

                buffer[i] = pixel[0];
                buffer[i + 1] = pixel[1];
                buffer[i + 2] = pixel[2];
                // no need to write to alpha
            }
        }
        b"IYU2" => {}
        _ => {}
    }
}
