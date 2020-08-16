use std::collections::{HashMap, HashSet};

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
    vy_slop: f32,
    color: Color,
}

impl Sprite {
    fn new(gfx: &Graphics, src: &image::DynamicImage, x: usize, y: usize, xx: f32, yy: f32, scale: u32, color: Color) -> Self {
        let (image, collider) = extract_sprite(gfx, src, x, y);
        Self {
            image,
            collider,
            loc: Vector::new(xx as f32, yy as f32),
            scale,
            velocity: Vector::new(0.0, 0.0),
            ground_contact: false,
            jumping: false,
            vy_slop: 0.0,
            color,
        }
    }

    fn overlap(&self, other: &Sprite) -> bool {
        let a = vek::geom::Rect::new(self.loc.x as i32, self.loc.y as i32, SPRITE_WIDTH as i32 * self.scale as i32, SPRITE_WIDTH as i32 * self.scale as i32);
        let b = vek::geom::Rect::new(other.loc.x as i32, other.loc.y as i32, SPRITE_WIDTH as i32 * other.scale as i32, SPRITE_WIDTH as i32 * other.scale as i32);
        if a.collides_with_rect(b) {
            let c = a.intersection(b);
            for x in c.x..c.x+c.w {
                for y in c.y..c.y+c.h {
                    let (dx, dy) = to_scale(x as i32 - self.loc.x as i32, y as i32 - self.loc.y as i32, self.scale);
                    let ai = dx as usize + dy as usize * SPRITE_WIDTH;
                    if self.collider[ai] {
                        let (dx, dy) = to_scale(x as i32 - other.loc.x as i32, y as i32 - other.loc.y as i32, other.scale);
                        let bi = dx as usize + dy as usize * SPRITE_WIDTH;
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
    sprites: HashMap<usize, Sprite>,
    potions: Vec<(usize, i32)>,
    characters: Vec<usize>,
    terrain_grids: HashMap<u32, HashMap<(i32, i32), bool>>,
    next_id: usize,
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
            sprites: HashMap::new(),
            terrain_grids: HashMap::new(),
            potions: vec![],
            characters: vec![],
            next_id: 0,
        }
    }

    fn add_sprite(&mut self, sprite: Sprite) -> usize {
        let id = self.next_id;
        self.next_id += 1;
        self.sprites.insert(id, sprite);
        id
    }

    fn add_potion(&mut self, sprite: Sprite, delta: i32) -> usize {
        let id = self.add_sprite(sprite);
        self.potions.push((id, delta));
        id
    }

    fn add_character(&mut self, sprite: Sprite) -> usize {
        let id = self.add_sprite(sprite);
        self.characters.push(id);
        id
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
        for sprite in self.sprites.values_mut() {
            sprite.velocity.y += 3.4 / FPS;
            sprite.loc.x += sprite.velocity.x * sprite.scale as f32;
            let mut blocked = false;
            let mut vy = sprite.velocity.y * sprite.scale as f32;
            let mut loc_y = sprite.loc.y;
            'outer: while vy.abs() >= 1.0 {
                loc_y += 1.0f32.copysign(sprite.velocity.y);
                vy -= 1.0f32.copysign(sprite.velocity.y);
                for x in 0..SPRITE_WIDTH {
                    for y in 0..SPRITE_WIDTH {
                        if sprite.collider[x as usize + y as usize * SPRITE_WIDTH as usize] {
                            for (scale, grid) in &self.terrain_grids {
                                for ddx in 0..sprite.scale {
                                    for ddy in 0..sprite.scale {
                                        let (dx, dy) = from_scale(x as i32, y as i32, sprite.scale);
                                        let x = sprite.loc.x as i32 + dx + ddx as i32;
                                        let y = loc_y as i32 + dy + ddy as i32;
                                        let (x, y) = to_scale(x , y, *scale);
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
                sprite.loc.y = loc_y;
            }
            if !blocked {
                if sprite.velocity.y.abs() >= 1.0 {
                    sprite.ground_contact = false;
                }
            } else if !sprite.ground_contact {
                sprite.velocity.y = 0.0;
                sprite.ground_contact = true;
                sprite.jumping = false;
            }
        }

        let mut scale_changes = vec![];
        let mut consumed = HashSet::new();
        for character_id in &self.characters {
            let character = &self.sprites[character_id];
            for (potion_id, delta) in &self.potions {
                if consumed.contains(potion_id) {
                    continue;
                }
                let potion = &self.sprites[potion_id];
                if character.overlap(potion) {
                    consumed.insert(*potion_id);
                    scale_changes.push((*character_id, *delta));
                }
            }
        }
        for potion_id in consumed {
            self.potions.retain(|(id, _)| *id != potion_id);
            self.sprites.remove(&potion_id);
        }
        for (character_id, delta) in scale_changes {
            let character = self.sprites.get_mut(&character_id).unwrap();
            character.scale = (character.scale as i32 + delta).max(1) as u32;
            character.loc.x -= SPRITE_WIDTH as f32 * delta as f32 * 0.5;
            character.loc.y -= SPRITE_WIDTH as f32 * delta as f32;
            let mut collisions = vec![];
            for x in 0..SPRITE_WIDTH {
                for y in 0..SPRITE_WIDTH {
                    if character.collider[x as usize + y as usize * SPRITE_WIDTH as usize] {
                        for (scale, grid) in &self.terrain_grids {
                            for ddx in 0..character.scale {
                                for ddy in 0..character.scale {
                                    let (dx, dy) = from_scale(x as i32, y as i32, character.scale);
                                    let x = character.loc.x as i32 + dx + ddx as i32;
                                    let y = character.loc.y as i32 + dy - ddy as i32;
                                    let (x, y) = to_scale(x, y, *scale);
                                    if let Some(true) = grid.get(&(x, y)) {
                                        collisions.push((*scale, (x, y)));
                                    }
                                }
                            }
                        }
                    }
                }
            }
            for (scale, (x,y)) in collisions {
                self.terrain_grids.get_mut(&scale).unwrap().remove(&(x,y));
            }
        }
    }

    fn draw(&self, gfx: &mut Graphics, x: i32, y: i32, width: u32, height: u32, scale: f32) {
        let mut pixels = vec![0; width as usize * height as usize * 3];
        for sprite in self.sprites.values() {
            for dx in 0..SPRITE_WIDTH*sprite.scale as usize {
                let lx = dx / sprite.scale as usize;
                let xx = sprite.loc.x as i32 + dx as i32;
                if xx < x || xx >= x+width as i32 {
                    continue
                }
                for dy in 0..SPRITE_WIDTH*sprite.scale as usize {
                    let yy = sprite.loc.y as i32 + dy as i32;
                    if yy < y || yy >= y+width as i32 {
                        continue
                    }
                    let ly = dy / sprite.scale as usize;
                    if sprite.collider[lx + ly*SPRITE_WIDTH] {
                        let i = ((xx - x) as usize + (yy -y) as usize * width as usize)*3;
                        if i < pixels.len() {
                            pixels[i] = (sprite.color.r * 255.0) as u8;
                            pixels[i+1] = (sprite.color.g * 255.0) as u8;
                            pixels[i+2] = (sprite.color.b * 255.0) as u8;
                        }
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
                                if lx >= 0 && lx < width as i32 && ly >= 0 && ly < height as i32 {
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
    let player_id = scene.add_character(Sprite::new(&gfx, &sprites, 31, 2, 300.0, 688.0, 7, Color::BLUE));
    for x in 0..10 {
        scene.add_potion(Sprite::new(&gfx, &sprites, 33, 13, (x as f32 * 2.0) *100.0, 688.0, 2, Color::RED), 1);
    }
    for x in 0..10 {
        scene.add_potion(Sprite::new(&gfx, &sprites, 33, 13, (x as f32 * 2.0 + 1.0) *100.0, 688.0, 2, Color::BLUE), -1);
    }

    for x in 0..20 {
        scene.add_terrain(&Sprite::new(&gfx, &sprites, 7, 5, (x*(SPRITE_WIDTH-2)*7) as f32, 800.0, 7, Color::BLUE));
    }
    for x in 0..100 {
        scene.add_terrain(&Sprite::new(&gfx, &sprites, 20, 15, (x*SPRITE_WIDTH*7) as f32, 590.0, 7, Color::BLUE));
    }

    let mut update_timer = Timer::time_per_second(FPS);
    let mut draw_timer = Timer::time_per_second(FPS);
    let mut camera = Vector::new(0.0, 0.0);
    loop {
        while let Some(e) = input.next_event().await {
            let vx = if input.key_down(Key::LShift) {
                92.0
            } else {
                46.0
            };
            match e {
                Event::KeyboardInput(e) => {
                    let player = scene.sprites.get_mut(&player_id).unwrap();
                    match e.key() {
                        Key::Right => {
                            if e.is_down() {
                                player.velocity.x = vx / FPS;
                            } else {
                                player.velocity.x = 0.0;
                            }
                        },
                        Key::Left => {
                            if e.is_down() {
                                player.velocity.x = -vx / FPS;
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
            let player = scene.sprites.get_mut(&player_id).unwrap();
            camera.x = player.loc.x - 1920.0/2.0;
            camera.y = player.loc.y - 1080.0/2.0;
            gfx.clear(Color::WHITE);
            let scale = player.scale as f32 / 8.0;
            scene.draw(&mut gfx, camera.x as i32, camera.y as i32, 1920, 1080, scale);
            gfx.present(&window)?;
        }
    }
}
