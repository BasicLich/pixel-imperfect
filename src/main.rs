use std::collections::HashMap;

use image::GenericImageView;

use quicksilver::{
    geom::{Rectangle, Vector},
    graphics::{Color, Graphics, Image, PixelFormat,},
    input::{Event, Key},
    Input, Window, Result, Settings, run,
    Timer,
};

const SPRITES:&[u8] = include_bytes!("../static/colored_transparent_packed.png");
const SPRITES_WIDTH:usize = 768;
const SPRITES_HEIGHT:usize = 352;
const SPRITE_WIDTH:usize = 16;
const PIXEL_CHUNK: u32 = 4;

const FPS: f32 = 60.0;

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
    velocity: Vector,
    ground_contact: bool,
    jumping: bool,
}

impl Sprite {
    fn new(gfx: &Graphics, src: &image::DynamicImage, x: usize, y: usize, xx: f32, yy: f32, scale: u32) -> Self {
        let (image, collider) = extract_sprite(gfx, src, x, y);
        Self {
            image,
            collider,
            loc: Vector::new(xx as f32, yy as f32),
            scale,
            velocity: Vector::new(0.0, 0.0),
            ground_contact: false,
            jumping: false,
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
    terrain_grids: HashMap<u32, HashMap<(i32, i32), bool>>,
}

fn to_scale(x: i32, y: i32, scale: u32) -> (i32, i32) {
    let x = x / scale as i32;
    let y = y / scale as i32;
    (x,y)
}

fn from_scale(x: i32, y: i32, scale: u32) -> (i32, i32) {
    let x = x * scale as i32;
    let y = y * scale as i32;
    (x,y)
}

impl Scene {
    fn new() -> Self {
        Self {
            sprites: vec![],
            terrain_grids: HashMap::new(),
        }
    }

    fn add_terrain(&mut self, sprite: &Sprite) {
        let x = sprite.loc.x as i32 / sprite.scale as i32;
        let y = sprite.loc.y as i32 / sprite.scale as i32;
        let grid = self.terrain_grids.entry(sprite.scale).or_insert_with(|| HashMap::new());
        for dx in 0..SPRITE_WIDTH {
            for dy in 0..SPRITE_WIDTH {
                if sprite.collider[dx + dy*SPRITE_WIDTH] {
                    grid.insert((x+dx as i32, y+dy as i32), true);
                }
            }
        }
    }

    fn step_physics(&mut self) {
        for sprite in &mut self.sprites {
            sprite.velocity.y += 3.4 / FPS;
            sprite.loc.x += sprite.velocity.x * sprite.scale as f32;
            let mut blocked = false;
            if sprite.velocity.y > 0.0 {
                'outer: for x in 0..SPRITE_WIDTH {
                    for y in 0..SPRITE_WIDTH {
                        if sprite.collider[x as usize + y as usize * SPRITE_WIDTH as usize] {
                            for (scale, grid) in &self.terrain_grids {
                                if *scale == sprite.scale {
                                    if let Some(true) = grid.get(&(x as i32 + sprite.loc.x as i32 / sprite.scale as i32,y as i32 + sprite.loc.y as i32/sprite.scale as i32)) {
                                        blocked = true;
                                        break 'outer;
                                    }
                                } else {
                                    let (dx, dy) = from_scale(x as i32, y as i32, sprite.scale);
                                    let x = sprite.loc.x as i32 + dx;
                                    let y = sprite.loc.y as i32 + dy;
                                    let (x, y) = to_scale(x, y, *scale);
                                    if let Some(true) = grid.get(&(x, y)) {
                                        blocked = true;
                                        break 'outer;
                                    }
                                }
                            }
                        }
                    }
                }
            }
            if !blocked {
                sprite.loc.y += sprite.velocity.y * sprite.scale as f32;
                sprite.ground_contact = false;
            } else if !sprite.ground_contact {
                sprite.velocity.y = 0.0;
                sprite.ground_contact = true;
                sprite.jumping = false;
            }
        }
    }

    fn draw(&self, gfx: &mut Graphics, x: i32, y: i32, width: u32, height: u32, scale: u32) {
        let mut pixels = vec![0; width as usize * height as usize * 3];
        for sprite in &self.sprites {
            for dx in 0..SPRITE_WIDTH*sprite.scale as usize {
                let lx = dx /sprite.scale as usize;
                let xx = sprite.loc.x as i32 + dx as i32;
                if xx < x || xx >= x+width as i32 {
                    continue
                }
                for dy in 0..SPRITE_WIDTH*sprite.scale as usize {
                    let yy = sprite.loc.y as i32 + dy as i32;
                    if yy < y || yy >= y+width as i32 {
                        continue
                    }
                    let ly = dy /sprite.scale as usize;
                    if sprite.collider[lx + ly*SPRITE_WIDTH] {
                        let i = ((xx - x) as usize + (yy -y) as usize * width as usize)*3;
                        pixels[i] = 0xff;
                        pixels[i+1] = 0x00;
                        pixels[i+2] = 0x00;
                    }
                }
            }
        }
        for (scale, grid) in &self.terrain_grids {
            for xx in x/ *scale as i32..(x+width as i32) / *scale as i32 {
                for yy in y/ *scale as i32..(y+height as i32)/ *scale as i32 {
                    if let Some(true) = grid.get(&(xx as i32, yy as i32)) {
                        for dx in 0..*scale as usize {
                            for dy in 0..*scale as usize {
                                let lx = xx * *scale as i32 - x + dx as i32;
                                let ly = yy * *scale as i32 - y + dy as i32;
                                let i = (lx + ly * width as i32) as usize * 3;
                                pixels[i] = 0x00;
                                pixels[i+1] = 0xff;
                                pixels[i+2] = 0x00;
                            }
                        }
                    }
                }
            }
        }


        let mut image = Image::from_raw(
            gfx,
            Some(&pixels),
            width,
            height,
            PixelFormat::RGB,
        ).unwrap();
        image.set_magnification(golem::TextureFilter::Nearest).unwrap();
        let region = Rectangle::new_sized(Vector::new(1920.0, 1080.0));
        gfx.draw_image(&image, region);

    }
}

async fn app(window: Window, mut gfx: Graphics, mut input: Input) -> Result<()> {
    let sprites = image::load(std::io::Cursor::new(SPRITES), image::ImageFormat::Png).unwrap();
    let mut scene = Scene::new();
    let player_id = 0;
    scene.sprites.push(Sprite::new(&gfx, &sprites, 31, 2, 100.0, 10.0, 16));
    scene.sprites.push(Sprite::new(&gfx, &sprites, 33, 13, 100.0, 10.0, 2));
    scene.sprites.push(Sprite::new(&gfx, &sprites, 34, 13, 100.0, 10.0, 2));

    let castle = Sprite::new(&gfx, &sprites, 2, 19, 100.0, 400.0, 7);
    for x in 0..100 {
        scene.add_terrain(&Sprite::new(&gfx, &sprites, 7, 5, (x*(SPRITE_WIDTH-2)*7) as f32, 800.0, 7));
    }

    let mut update_timer = Timer::time_per_second(FPS);
    let mut draw_timer = Timer::time_per_second(FPS);
    loop {
        while let Some(e) = input.next_event().await {
            match e {
                Event::KeyboardInput(e) => {
                    let player = &mut scene.sprites[player_id];
                    match e.key() {
                        Key::Right => {
                            if e.is_down() {
                                player.velocity.x = 23.0 / FPS;
                            } else {
                                player.velocity.x = 0.0;
                            }
                        },
                        Key::Left => {
                            if e.is_down() {
                                player.velocity.x = -23.0 / FPS;
                            } else {
                                player.velocity.x = 0.0;
                            }
                        },
                        Key::Up => {
                            if e.is_down() {
                                if player.ground_contact {
                                    player.jumping = true;
                                    player.velocity.y = -68.0 / FPS;
                                }
                            } else {
                                if !player.ground_contact && player.jumping {
                                    player.velocity.y = player.velocity.y.max(-2.0);
                                }
                            }
                        },
                        Key::A => {
                            if e.is_down() {
                                player.scale += 1;
                            }
                        },
                        Key::O => {
                            if e.is_down() {
                                player.scale -= 1;
                            }
                        },
                        _ => (),
                    }
                },
                _ => (),
            }
        }
        while update_timer.tick() {
            scene.step_physics();
        }
        if draw_timer.exhaust().is_some() {
            gfx.clear(Color::WHITE);
            scene.draw(&mut gfx, 0, 0, 1920, 1080, 1);
            //scene.draw_terrain(&mut gfx, 1920, 1080);
            gfx.present(&window)?;
        }
    }
}
