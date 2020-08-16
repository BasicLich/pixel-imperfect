use image::GenericImageView;

use quicksilver::{
    geom::{Rectangle, Vector},
    graphics::{Color, Graphics, Image, PixelFormat,},
    input::{Event, Key},
    Input, Window, Result, Settings, run,
};

const SPRITES:&[u8] = include_bytes!("../static/colored_transparent_packed.png");
const SPRITES_WIDTH:usize = 768;
const SPRITES_HEIGHT:usize = 352;
const SPRITE_WIDTH:usize = 16;
const PIXEL_CHUNK: u32 = 4;

fn main() {
    run(
        Settings {
            title: "Square Example",
            size: Vector::new(1920.0, 1080.0),
            fullscreen: true,
            ..Settings::default()
        },
        app,
    );
}

fn extract_sprite(gfx: &Graphics, src: &image::DynamicImage, x: usize, y: usize) -> (Image, [bool; SPRITE_WIDTH*SPRITE_WIDTH]) {
    let mut pixels = vec![0; SPRITE_WIDTH*SPRITE_WIDTH*4];
    let mut collider = [false; SPRITE_WIDTH*SPRITE_WIDTH];
    let x = x * SPRITE_WIDTH;
    let y = y * SPRITE_WIDTH;
    for dx in 0..SPRITE_WIDTH {
        for dy in 0..SPRITE_WIDTH {
            let i = dx*4 + dy*4 * SPRITE_WIDTH;
            let p = src.get_pixel((x+dx) as u32, (y+dy) as u32);
            pixels[i] = p.0[0];
            pixels[i+1] = p.0[1];
            pixels[i+2] = p.0[2];
            pixels[i+3] = p.0[3];
            if p.0[3] > 0 {
                collider[dx + dy*SPRITE_WIDTH] = true;
            }
        }
    }
    let mut image = Image::from_raw(
        gfx,
        Some(&pixels),
        SPRITE_WIDTH as u32,
        SPRITE_WIDTH as u32,
        PixelFormat::RGBA,
    ).unwrap();
    image.set_magnification(golem::TextureFilter::Nearest).unwrap();
    (image, collider)
}

struct Sprite {
    image: Image,
    collider: [bool; SPRITE_WIDTH*SPRITE_WIDTH],
    loc: Vector,
    scale: u32,
}

impl Sprite {
    fn new(gfx: &Graphics, src: &image::DynamicImage, x: usize, y: usize, xx: f32, yy: f32, scale: u32) -> Self {
        let (image, collider) = extract_sprite(gfx, src, x, y);
        Self {
            image,
            collider,
            loc: Vector::new(xx as f32, yy as f32),
            scale,
        }
    }

    fn draw(&self, gfx: &mut Graphics) {
        let region = Rectangle::new(self.loc, Vector::new(SPRITE_WIDTH as f32 * self.scale as f32, SPRITE_WIDTH as f32 * self.scale as f32));
        gfx.draw_image(&self.image, region);
    }

    fn overlap(&self, other: &Sprite) -> bool {
        let a = vek::geom::Rect::new(self.loc.x as i32, self.loc.y as i32, SPRITE_WIDTH as i32, SPRITE_WIDTH as i32);
        let b = vek::geom::Rect::new(other.loc.x as i32, other.loc.y as i32, SPRITE_WIDTH as i32, SPRITE_WIDTH as i32);
        if a.collides_with_rect(b) {
            let c = a.intersection(b);
            for x in c.x..c.x+c.w {
                for y in c.y..c.y+c.h {
                    let ai = (x as usize - self.loc.x as usize) + (y as usize - self.loc.y as usize) * SPRITE_WIDTH;
                    if self.collider[ai] {
                        let bi = (x as usize - other.loc.x as usize) + (y as usize - other.loc.y as usize) * SPRITE_WIDTH;
                        if other.collider[bi] {
                            return true
                        }
                    }
                }
            }
        }
        false
    }
}

struct Scene {
    sprites: Vec<Sprite>,
    terrain: Vec<Sprite>,
}

impl Scene {
    fn new() -> Self {
        Self {
            sprites: vec![],
            terrain: vec![],
        }
    }

    fn step_physics(&mut self) {
        for sprite in &mut self.sprites {
            if !self.terrain.iter().any(|t| t.overlap(sprite)) {
                sprite.loc.y += sprite.scale as f32;
            }
        }
    }
}

async fn app(window: Window, mut gfx: Graphics, mut input: Input) -> Result<()> {
    let sprites = image::load(std::io::Cursor::new(SPRITES), image::ImageFormat::Png).unwrap();
    let mut scene = Scene::new();
    let player_id = 0;
    scene.sprites.push(Sprite::new(&gfx, &sprites, 31, 2, 100.0, 10.0, 8));
    scene.sprites.push(Sprite::new(&gfx, &sprites, 33, 13, 100.0, 10.0, 2));
    scene.sprites.push(Sprite::new(&gfx, &sprites, 34, 13, 100.0, 10.0, 2));

    scene.terrain.push(Sprite::new(&gfx, &sprites, 2, 19, 100.0, 400.0, 16));
    loop {
        while let Some(e) = input.next_event().await {
            match e {
                Event::KeyboardInput(e) => {
                    if e.is_down() {
                        match e.key() {
                            Key::Right => {
                                scene.sprites[player_id].loc.x += scene.sprites[player_id].scale as f32;
                            },
                            Key::Left => {
                                scene.sprites[player_id].loc.x -= scene.sprites[player_id].scale as f32;
                            },
                            Key::Up => {
                                scene.sprites[player_id].scale += 1;
                            },
                            Key::Down => {
                                scene.sprites[player_id].scale = (scene.sprites[player_id].scale - 1).max(1);
                            },
                            _ => (),
                        }
                    }
                },
                _ => (),
            }
        }
        scene.step_physics();
        gfx.clear(Color::WHITE);
        for s in &scene.sprites {
            s.draw(&mut gfx);
        }
        for s in &scene.terrain {
            s.draw(&mut gfx);
        }
        gfx.present(&window)?;
    }
}
