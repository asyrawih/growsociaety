//! The wardrobe's character preview: the local player's current look at the rest pose, composed
//! into one image: the rig's body layers in the local player's skin tone (`player::RigImages`),
//! then what is worn, per assets/README.md and `player::rig`. Rebuilt only when an `Equipped`
//! event for the local player arrives or the skinned body images change (which includes startup).
//! The HUD shows `WardrobePreview::image`.

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use game_shared::items::{SLOT_COUNT, Slot, item};
use game_shared::rules::Event;
use rig::Part;

use crate::player::{RigImages, rig};
use crate::state::{Game, GameEvent, Phase};

/// Px. The rig layers are square canvases.
const RIG_CANVAS: u32 = rig::CANVAS as u32;
/// Px. The preview canvas is wider than the rig so wings and the held pickaxe fit; the rig sits
/// `RIG_OFFSET_X` from its left edge, so it is centred, and on the bottom edge.
pub const PREVIEW_SIZE: UVec2 = UVec2::new(64, 48);
const RIG_OFFSET_X: u32 = (PREVIEW_SIZE.x - RIG_CANVAS) / 2;
/// Px. Empty rows of the preview canvas below the feet.
pub const PREVIEW_BELOW_FEET: u32 = RIG_CANVAS - rig::FEET_Y as u32;

/// The preview image (reserved at startup, filled once its sources have loaded) and the sources
/// of the build in progress, back to front.
#[derive(Resource)]
pub struct WardrobePreview {
    pub image: Handle<Image>,
    pending: Vec<(Handle<Image>, Place)>,
}

impl FromWorld for WardrobePreview {
    fn from_world(world: &mut World) -> Self {
        let image = world.resource::<Assets<Image>>().reserve_handle();
        Self {
            image,
            pending: Vec::new(),
        }
    }
}

/// Where a layer's image comes from.
#[derive(Clone, PartialEq, Debug)]
enum Source {
    /// The skinned body layer of a rig part (`RigImages::skinned`, indexed like `Part::ALL`).
    Body(usize),
    /// An asset path.
    File(String),
}

/// Where a source layer goes on the preview canvas.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Place {
    /// A full rig canvas, at the rig's position.
    Rig,
    /// The held item, turned and put in the front hand.
    Held,
}

pub struct WardrobePlugin;

impl Plugin for WardrobePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<WardrobePreview>().add_systems(
            Update,
            (
                requeue_preview,
                build_preview.run_if(|preview: Res<WardrobePreview>| !preview.pending.is_empty()),
            )
                .chain()
                .in_set(Phase::View),
        );
    }
}

/// Where assets are read from, as Bevy resolves it (see `main.rs`).
fn asset_root() -> std::path::PathBuf {
    bevy::asset::io::file::FileAssetReader::get_base_path().join(crate::asset_dir())
}

/// The layers of a look, back to front. `worn` holds one item folder name per slot; `has_file`
/// says whether an asset path exists (an item only has the parts it touches).
fn layer_sources(
    worn: &[Option<&str>; SLOT_COUNT],
    has_file: impl Fn(&str) -> bool,
) -> Vec<(Source, Place)> {
    let mut layers = Vec::new();
    if let Some(wings) = worn[Slot::Back as usize] {
        let path = format!("items/{wings}/{}", rig::WINGS_FILE);
        layers.push((Source::File(path), Place::Rig));
    }
    for (index, part) in Part::ALL.into_iter().enumerate() {
        layers.push((Source::Body(index), Place::Rig));
        for slot in rig::STACK {
            let Some(name) = worn[slot as usize] else {
                continue;
            };
            let path = format!("items/{name}/{}.png", part.name());
            if has_file(&path) {
                layers.push((Source::File(path), Place::Rig));
            }
        }
    }
    if let Some(held) = worn[Slot::Hand as usize] {
        let path = format!("items/{held}/{}", rig::HELD_FILE);
        layers.push((Source::File(path), Place::Held));
    }
    layers
}

fn requeue_preview(
    mut preview: ResMut<WardrobePreview>,
    game: Res<Game>,
    assets: Res<AssetServer>,
    body: Option<Res<RigImages>>,
    mut events: MessageReader<GameEvent>,
) {
    let mine = events.read().any(
        |GameEvent(event)| matches!(*event, Event::Equipped { player, .. } if player == game.1),
    );
    // `RigImages` reads as changed when it is added and whenever the skin tone is refilled.
    let Some(body) = body.filter(|body| mine || body.is_changed()) else {
        return;
    };
    let Some(me) = game.me() else { return };
    let worn = me.equipped.map(|id| id.and_then(item).map(|def| def.name));
    let root = asset_root();
    let layers = layer_sources(&worn, |path| root.join(path).is_file());
    preview.pending = layers
        .into_iter()
        .filter_map(|(source, place)| {
            let handle = match source {
                Source::Body(index) => body.skinned.get(index)?.clone(),
                Source::File(path) => assets.load(path),
            };
            Some((handle, place))
        })
        .collect();
}

/// Composes the preview once every source has loaded. A source that fails to load is left out,
/// with a warning.
fn build_preview(
    assets: Res<AssetServer>,
    mut images: ResMut<Assets<Image>>,
    mut preview: ResMut<WardrobePreview>,
) {
    preview.pending.retain(|(source, _)| {
        let failed = assets.load_state(source).is_failed();
        if failed {
            warn!("wardrobe preview: {:?} failed to load", source.path());
        }
        !failed
    });
    let layers: Option<Vec<(Image, Place)>> = preview
        .pending
        .iter()
        .map(|(source, place)| Some((rgba8(images.get(source)?)?, *place)))
        .collect();
    let Some(layers) = layers else {
        return; // still loading
    };
    let image = compose(&layers);
    preview.pending.clear();
    if let Err(error) = images.insert(&preview.image, image) {
        warn!("wardrobe preview: {error}");
    }
}

fn rgba8(image: &Image) -> Option<Image> {
    match image.texture_descriptor.format {
        TextureFormat::Rgba8UnormSrgb => Some(image.clone()),
        _ => image.convert(TextureFormat::Rgba8UnormSrgb),
    }
}

/// Stacks RGBA8 layers back to front onto a `PREVIEW_SIZE` canvas.
fn compose(layers: &[(Image, Place)]) -> Image {
    let (width, height) = (PREVIEW_SIZE.x as usize, PREVIEW_SIZE.y as usize);
    let mut canvas = vec![0u8; width * height * 4];
    let (sin, cos) = (-rig::HELD_ANGLE).sin_cos();
    let (grip, hand) = (Vec2::from(rig::HELD_GRIP), Vec2::from(rig::HAND));
    for (layer, place) in layers {
        let Some(data) = layer.data.as_deref() else {
            continue;
        };
        let size = layer.size();
        let (src_w, src_h) = (size.x as usize, size.y as usize);
        if data.len() != src_w * src_h * 4 {
            continue;
        }
        let source_pixel = |x: usize, y: usize| -> Option<&[u8]> {
            let (sx, sy) = match place {
                Place::Rig => (
                    x.checked_sub(RIG_OFFSET_X as usize)?,
                    y.checked_sub((PREVIEW_SIZE.y - RIG_CANVAS) as usize)?,
                ),
                Place::Held => {
                    // Undo the turn: canvas pixel centre -> icon point, y up while rotating.
                    let rig = Vec2::new(
                        x as f32 + 0.5 - RIG_OFFSET_X as f32,
                        y as f32 + 0.5 - (PREVIEW_SIZE.y - RIG_CANVAS) as f32,
                    );
                    let d = Vec2::new(rig.x - hand.x, hand.y - rig.y);
                    let s = Vec2::new(d.x * cos - d.y * sin, d.x * sin + d.y * cos);
                    let icon = Vec2::new(grip.x + s.x, grip.y - s.y);
                    if icon.x < 0.0 || icon.y < 0.0 {
                        return None;
                    }
                    (icon.x as usize, icon.y as usize)
                }
            };
            if sx >= src_w || sy >= src_h {
                return None;
            }
            data.get((sy * src_w + sx) * 4..)?.get(..4)
        };
        for y in 0..height {
            for x in 0..width {
                if let Some(src) = source_pixel(x, y) {
                    let at = (y * width + x) * 4;
                    blend_over(&mut canvas[at..at + 4], src);
                }
            }
        }
    }
    Image::new(
        Extent3d {
            width: PREVIEW_SIZE.x,
            height: PREVIEW_SIZE.y,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        canvas,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    )
}

/// Straight-alpha "src over dst" for one RGBA8 pixel.
fn blend_over(dst: &mut [u8], src: &[u8]) {
    let (Some(&sa), Some(&da)) = (src.get(3), dst.get(3)) else {
        return;
    };
    let (sa, da) = (u32::from(sa), u32::from(da));
    let out_a = sa + da * (255 - sa) / 255;
    if out_a == 0 {
        return;
    }
    for (d, &s) in dst.iter_mut().zip(src).take(3) {
        let mixed = (u32::from(s) * sa + u32::from(*d) * da * (255 - sa) / 255) / out_a;
        *d = mixed.min(255) as u8;
    }
    dst[3] = out_a.min(255) as u8;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(side: u32, pixels: &[(usize, usize, [u8; 4])]) -> Image {
        let side_px = side as usize;
        let mut data = vec![0u8; side_px * side_px * 4];
        for &(x, y, rgba) in pixels {
            data[(y * side_px + x) * 4..][..4].copy_from_slice(&rgba);
        }
        Image::new(
            Extent3d {
                width: side,
                height: side,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            data,
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::default(),
        )
    }

    #[test]
    fn layers_go_wings_body_clothes_then_the_held_item() {
        let mut worn = [None; SLOT_COUNT];
        worn[Slot::Hat as usize] = Some("hat_x");
        worn[Slot::Hair as usize] = Some("hair_x");
        worn[Slot::Back as usize] = Some("wings_x");
        worn[Slot::Hand as usize] = Some("pickaxe_x");
        let layers = layer_sources(&worn, |path| path.ends_with("/head.png"));
        let file = |path: &str| Source::File(path.to_string());
        let body = |part: Part| Source::Body(Part::ALL.iter().position(|&p| p == part).unwrap());
        let at = |source: Source| layers.iter().position(|(s, _)| *s == source).unwrap();
        assert_eq!(at(file("items/wings_x/back.png")), 0);
        assert_eq!(at(body(Part::ArmBack)), 1);
        // Inside the head: body, then hair, then hat (over hair).
        assert!(at(body(Part::Head)) < at(file("items/hair_x/head.png")));
        assert!(at(file("items/hair_x/head.png")) < at(file("items/hat_x/head.png")));
        assert!(at(file("items/hat_x/head.png")) < at(body(Part::ArmFront)));
        assert_eq!(
            layers.last(),
            Some(&(file("items/pickaxe_x/icon.png"), Place::Held))
        );
    }

    #[test]
    fn compose_stacks_back_to_front_and_puts_the_grip_in_the_hand() {
        let red = [255, 0, 0, 255];
        let blue = [0, 0, 255, 255];
        let back = image(RIG_CANVAS, &[(10, 10, red), (11, 10, red)]);
        let front = image(RIG_CANVAS, &[(10, 10, blue)]);
        // A 3x3 blob around the grip of a 32x32 icon.
        let [gx, gy] = rig::HELD_GRIP.map(|v| v as usize);
        let blob: Vec<_> = (gx - 1..=gx + 1)
            .flat_map(|x| (gy - 1..=gy + 1).map(move |y| (x, y, red)))
            .collect();
        let held = image(32, &blob);
        let out = compose(&[(back, Place::Rig), (front, Place::Rig), (held, Place::Held)]);
        assert_eq!(out.size(), PREVIEW_SIZE);
        let data = out.data.unwrap();
        let top = (PREVIEW_SIZE.y - RIG_CANVAS) as usize;
        let pixel = |x: usize, y: usize| {
            let at = ((y + top) * PREVIEW_SIZE.x as usize + x + RIG_OFFSET_X as usize) * 4;
            data[at..at + 4].to_vec()
        };
        assert_eq!(pixel(10, 10), blue); // front over back
        assert_eq!(pixel(11, 10), red);
        assert_eq!(pixel(0, 0)[3], 0);
        let [hx, hy] = rig::HAND.map(|v| v as usize);
        assert_eq!(pixel(hx, hy), red);
    }
}
