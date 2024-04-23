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
    image: Handle<Image>,
    task: Option<Task<()>>,
    decoder: Arc<Mutex<Decoder>>,
}

pub struct Decoder {
    buffer: Vec<u8>,
    stream: Stream<'static>,
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
            image,
            decoder: Arc::new(Mutex::new(Decoder {
                buffer: buffer2,
                stream,
            })),
            task: None,
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

fn present(mut devices: Query<&mut V4lDevice>, mut images: ResMut<Assets<Image>>) {
    for mut device in devices.iter_mut() {
        let Some(mut task_status) = device.task.as_mut() else {
            continue;
        };

        if let Some(()) = futures::check_ready(&mut task_status) {
            let Some(image) = images.get_mut(device.image.clone()) else {
                continue;
            };

            if let Ok(mut decoder) = device.decoder.lock() {
                std::mem::swap(&mut image.data, &mut decoder.buffer);
            }

            device.task = None;
        }
    }
}

fn decode(mut devices: Query<&mut V4lDevice>, mut images: ResMut<Assets<Image>>) {
    for mut device in devices.iter_mut() {
        let Some(image) = images.get_mut(device.image.clone()) else {
            continue;
        };

        // task is unfinished
        if device.task.is_some() {
            continue;
        };

        let fourcc = device.format.fourcc.repr;
        let size = image.width() * image.height() * 4;
        let decoder = device.decoder.clone();
        let task = ComputeTaskPool::get()
            .spawn(async move { unsafe { decode_to_rgba(decoder, &fourcc, size as usize) } });

        device.task = Some(task);
    }
}

unsafe fn decode_to_rgba(decoder: Arc<Mutex<Decoder>>, fourcc: &[u8; 4], size: usize) {
    let Ok(mut decoder) = decoder.lock() else {
        return;
    };

    // SAFETY:
    // mutex is locked for the decoder so nothing else can write to this buffer
    let buffer: *mut Vec<u8> = &mut decoder.buffer;
    let Ok((buf, _)) = decoder.stream.next() else {
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

                (*buffer)[i..i + 3].clone_from_slice(&pixel);
            }
        }
        b"IYU2" => {}
        _ => {}
    }
}
