use std::sync::{Arc, Mutex};
use std::time::Duration;

use bevy::prelude::*;
use bevy::render::render_asset::RenderAssetUsages;
use bevy::render::render_resource::{
    Extent3d, TextureDescriptor, TextureDimension, TextureFormat, TextureUsages,
};
use bevy::tasks::{ComputeTaskPool, Task};
use bevy::utils::futures;
use ffimage::color::Rgb;
use ffimage::iter::{BytesExt, ColorConvertExt, PixelsExt};
use ffimage_yuv::yuv::Yuv;
use ffimage_yuv::yuv422::Yuv422;
use thiserror::Error;
use v4l::io::mmap::Stream;
use v4l::io::traits::{CaptureStream, OutputStream};
use v4l::prelude::*;
use v4l::video::Capture;

const BUFFER_COUNT: u32 = 4;

type Result<T> = std::result::Result<T, Error>;

#[derive(Error, Debug)]
pub enum Error {
    #[error("v4l device unavailable")]
    Io(#[from] std::io::Error),
}

#[derive(Component)]
pub struct Input(Device);

impl Input {
    /// Creates a V4lDevice for encoding a bevy image into v4l
    pub fn new(device_id: usize, images: &mut ResMut<Assets<Image>>) -> Result<Self> {
        let dev = v4l::Device::new(device_id)?;
        let format = dev.format()?;
        let stream = MmapStream::with_buffers(&dev, v4l::buffer::Type::VideoCapture, BUFFER_COUNT)?;

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

        Ok(Self(crate::Device {
            id: device_id,
            format,
            image,
            size,
            io: Arc::new(Mutex::new(Io {
                buffer: buffer2,
                stream,
            })),
            task: None,
            dev,
        }))
    }

    pub fn clone_image(&mut self, images: &mut ResMut<Assets<Image>>) -> Handle<Image> {
        let buffer = vec![255_u8; (self.0.size.width * self.0.size.height * 4) as usize];
        images.add(Image {
            data: buffer,
            texture_descriptor: TextureDescriptor {
                label: None,
                size: self.0.size,
                dimension: TextureDimension::D2,
                format: TextureFormat::Rgba8UnormSrgb,
                mip_level_count: 1,
                sample_count: 1,
                usage: TextureUsages::TEXTURE_BINDING
                    | TextureUsages::COPY_DST
                    | TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            },
            asset_usage: RenderAssetUsages::all(),
            ..default()
        })
    }

    /// Handle to bevy image
    pub fn image(&self) -> &Handle<Image> {
        &self.0.image
    }

    /// ID of the v4l video device (/dev/video{id})
    pub fn id(&self) -> usize {
        self.0.id
    }

    pub fn format(&self) -> Format {
        Format(self.0.format)
    }

    pub fn size(&self) -> Extent3d {
        self.0.size
    }
}

#[derive(Component)]
pub struct Output(Device);

impl Output {
    /// Creates a V4lDevice for encoding a bevy image into v4l
    pub fn new(device_id: usize, image: Handle<Image>, format: Format) -> Result<Self> {
        let format = format.0;
        let dev = v4l::Device::new(device_id)?;

        let _ = v4l::video::Output::set_format(&dev, &format)?;

        let stream = MmapStream::with_buffers(&dev, v4l::buffer::Type::VideoOutput, BUFFER_COUNT)?;

        let size = Extent3d {
            width: format.width,
            height: format.height,
            depth_or_array_layers: 1,
        };

        let buffer1 = vec![255_u8; (size.width * size.height * 4) as usize];
        let buffer2 = buffer1.clone();

        Ok(Self(crate::Device {
            id: device_id,
            format,
            image,
            size,
            io: Arc::new(Mutex::new(Io {
                buffer: buffer2,
                stream,
            })),
            task: None,
            dev,
        }))
    }

    /// Handle to bevy image
    pub fn image(&self) -> &Handle<Image> {
        &self.0.image
    }

    /// ID of the v4l video device (/dev/video{id})
    pub fn id(&self) -> usize {
        self.0.id
    }

    pub fn format(&self) -> Format {
        Format(self.0.format)
    }

    pub fn size(&self) -> Extent3d {
        self.0.size
    }
}

//TODO: add a way to construct a format
pub struct Format(v4l::Format);

/// Handle to a v4l Device
#[allow(dead_code)]
#[derive(Component)]
struct Device {
    id: usize,
    format: v4l::Format,
    image: Handle<Image>,
    size: Extent3d,
    task: Option<Task<()>>,
    io: Arc<Mutex<Io>>,
    /// NOTE: dropping this might panic :)
    dev: v4l::Device,
}

/// IO Data used in a bevy task
struct Io {
    /// Internal buffer for a frame.
    /// On:
    /// - input: double buffered with bevy Image.data
    /// - output: copy of Image.data
    buffer: Vec<u8>,
    stream: Stream<'static>,
}

pub struct V4lPlugin;
impl Plugin for V4lPlugin {
    fn build(&self, app: &mut bevy::prelude::App) {
        app.add_systems(PreUpdate, spawn_io_tasks)
            .add_systems(Update, poll_io_tasks);
    }
}

fn poll_io_tasks(
    mut inputs: Query<&mut Input>,
    mut outputs: Query<&mut Output>,
    mut images: ResMut<Assets<Image>>,
) {
    for mut input in inputs.iter_mut() {
        let device = &mut input.0;
        let Some(mut task_status) = device.task.as_mut() else {
            continue;
        };

        if let Some(()) = futures::check_ready(&mut task_status) {
            let Some(image) = images.get_mut(device.image.clone()) else {
                continue;
            };

            if let Ok(mut io) = device.io.lock() {
                std::mem::swap(&mut image.data, &mut io.buffer);
                tracing::debug!("input capture buffer swapped");
            }

            device.task = None;
        }
    }

    for mut output in outputs.iter_mut() {
        let device = &mut output.0;
        let Some(mut task_status) = device.task.as_mut() else {
            continue;
        };

        if let Some(()) = futures::check_ready(&mut task_status) {
            let Some(image) = images.get_mut(device.image.clone()) else {
                continue;
            };

            if let Ok(mut io) = device.io.lock() {
                io.buffer = image.data.clone();
                tracing::debug!("frame buffer cloned to io");
            }

            device.task = None;
        }
    }
}

fn spawn_io_tasks(
    mut inputs: Query<&mut Input>,
    mut outputs: Query<&mut Output>,
    mut images: ResMut<Assets<Image>>,
) {
    for mut input in inputs.iter_mut() {
        let device = &mut input.0;
        let Some(image) = images.get_mut(device.image.clone()) else {
            return;
        };

        // task is unfinished
        if device.task.is_some() {
            return;
        };

        let fourcc = device.format.fourcc.repr;
        let size = image.width() * image.height() * 4;
        let io = device.io.clone();
        let task = ComputeTaskPool::get().spawn(async move {
            if let Ok(mut io) = io.lock() {
                let _ = stream_read(&mut io, &fourcc, size as usize);
            };
        });

        device.task = Some(task);
    }

    for mut output in outputs.iter_mut() {
        let device = &mut output.0;

        let Some(image) = images.get_mut(device.image.clone()) else {
            return;
        };

        // task is unfinished
        if device.task.is_some() {
            return;
        };

        let fourcc = device.format.fourcc.repr;
        let size = image.width() * image.height() * 4;
        let io = device.io.clone();
        let task = ComputeTaskPool::get().spawn(async move {
            if let Ok(mut io) = io.lock() {
                let _ = stream_write(&mut io, &fourcc, size as usize);
            };
        });

        device.task = Some(task);
    }
}

fn stream_read(io: &mut Io, fourcc: &[u8; 4], size: usize) -> Result<()> {
    let (buf, _) = CaptureStream::next(&mut io.stream)?;

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

                io.buffer[i..i + 3].clone_from_slice(&pixel);
            }
        }
        b"IYU2" => {}
        _ => {}
    }
    Ok(())
}

fn stream_write(io: &mut Io, fourcc: &[u8; 4], size: usize) -> Result<()> {
    let (buf, buf_meta) = OutputStream::next(&mut io.stream)?;

    // TODO: support other formats
    match fourcc {
        b"YUYV" => {
            io.buffer
                .chunks_exact(8)
                .map(|rgb| {
                    [
                        // buffer is rgba, skip alpha channel
                        Yuv::<u8>::from(Rgb::<u8>(rgb[0..3].try_into().unwrap())),
                        Yuv::<u8>::from(Rgb::<u8>(rgb[4..7].try_into().unwrap())),
                    ]
                })
                .colorconvert::<Yuv422<u8, 0, 2, 1, 3>>()
                .bytes()
                .write(&mut buf.iter_mut());

            buf_meta.field = 0;
            buf_meta.bytesused = size as u32 * 3;
        }
        b"IYU2" => {}
        _ => {}
    }
    Ok(())
}
