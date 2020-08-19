#![feature(clamp)]
use indexmap::{IndexMap, IndexSet};

use image::GenericImageView;

use quicksilver::{
    geom::{Rectangle, Vector},
    graphics::{Color, Graphics, Image, PixelFormat,},
    input::{Event, Key},
    Input, Window, Result, Settings, run,
    Timer,
};

const SPRITES:&[u8] = include_bytes!("../static/monochrome_transparent_packed.png");
const SPRITES_WIDTH:usize = 768;
const SPRITES_HEIGHT:usize = 352;
const SPRITE_WIDTH:usize = 16;
const PIXEL_CHUNK: u32 = 4;
const MAX_SCALE: usize = 18;
const SCALE_CHANGE_TIMEOUT: f32 = 1.0;

const FOREGROUND_COLOR: Color = Color { r: 100.0/255.0, g: 200.0/255.0, b: 100.0/255.0, a: 1.0 };
const BACKGROUND_COLOR: Color = Color { r: 50.0/255.0, g: 50.0/255.0, b: 100.0/255.0, a: 1.0 };
const TERRAIN_COLOR: Color = Color { r: 255.0, g: 255.0, b: 255.0, a: 1.0 };

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

fn extract_sprite(src: &image::DynamicImage, x: usize, y: usize) -> [bool; SPRITE_WIDTH*SPRITE_WIDTH] {
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
    collider
}

struct Sprite {
    is_player: bool,
    collider: [bool; SPRITE_WIDTH*SPRITE_WIDTH],
    loc: Vector,
    x_scale: u32,
    y_scale: u32,
    velocity: Vector,
    ground_contact: bool,
    jumping: bool,
    vy_slop: f32,
    color: Color,
    scale_timer: Option<f32>,
    pending_x_scale: i32,
    pending_y_scale: i32,
    sleep_timer: f32,
}

impl Sprite {
    fn new(src: &image::DynamicImage, x: usize, y: usize, xx: f32, yy: f32, x_scale: u32, y_scale: u32, color: Color) -> Self {
        let collider = extract_sprite(src, x, y);
        Sprite::from_collider(collider, xx, yy, x_scale, y_scale, color)
    }

    fn from_collider(collider: [bool; SPRITE_WIDTH*SPRITE_WIDTH], xx: f32, yy: f32, x_scale: u32, y_scale: u32, color: Color) -> Self {
        Self {
            is_player: false,
            collider,
            loc: Vector::new(xx as f32, yy as f32),
            x_scale,
            y_scale,
            velocity: Vector::new(0.0, 0.0),
            ground_contact: false,
            jumping: false,
            vy_slop: 0.0,
            color,
            scale_timer: None,
            pending_x_scale: 0,
            pending_y_scale: 0,
            sleep_timer: 0.0,
        }
    }

    fn overlap(&self, other: &Sprite) -> bool {
        let a = vek::geom::Rect::new(self.loc.x as i32, self.loc.y as i32, SPRITE_WIDTH as i32 * self.x_scale as i32, SPRITE_WIDTH as i32 * self.y_scale as i32);
        let b = vek::geom::Rect::new(other.loc.x as i32, other.loc.y as i32, SPRITE_WIDTH as i32 * other.x_scale as i32, SPRITE_WIDTH as i32 * other.y_scale as i32);
        if a.collides_with_rect(b) {
            let c = a.intersection(b);
            for x in c.x..c.x+c.w {
                for y in c.y..c.y+c.h {
                    let (dx, dy) = to_scale(x as i32 - self.loc.x as i32, y as i32 - self.loc.y as i32, self.x_scale, self.y_scale);
                    let ai = dx as usize + dy as usize * SPRITE_WIDTH;
                    if self.collider[ai] {
                        let (dx, dy) = to_scale(x as i32 - other.loc.x as i32, y as i32 - other.loc.y as i32, other.x_scale, other.y_scale);
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

    fn image(&self, gfx: &Graphics) -> Image {
        let mut pixels = [0; SPRITE_WIDTH*SPRITE_WIDTH*4];
        for (i, src) in self.collider.iter().enumerate() {
            if *src {
                pixels[i*4] = (self.color.r * 255.0).clamp(0.0, 255.0) as u8;
                pixels[i*4+1] = (self.color.g * 255.0).clamp(0.0, 255.0) as u8;
                pixels[i*4+2] = (self.color.b * 255.0).clamp(0.0, 255.0) as u8;
                pixels[i*4+3] = 0xff;
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
        image
    }
}

const LEAF_SIZE: usize = 64;
struct CollisionTree {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    free_pixels: u32,
    children: Option<Vec<CollisionTree>>,
    grid: Option<[bool; LEAF_SIZE*LEAF_SIZE]>,
}

impl CollisionTree {
    fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        let width = ((width as f32 / 4.0).ceil() * 4.0) as u32;
        let height = ((height as f32 / 4.0).ceil() * 4.0) as u32;
        Self {
            x,
            y,
            width,
            height,
            free_pixels: width * height,
            children: None,
            grid: None,
        }
    }

    fn clear(&mut self) {
        self.free_pixels = self.width*self.height;
        self.children.take();
        self.grid.take();
    }

    fn insert(&mut self, x: i32, y: i32) -> std::result::Result<bool, ()> {
        if x < self.x || x >= self.x+self.width as i32 || y < self.y || y >= self.y+self.height as i32 {
            return Err(());
        }
        if self.free_pixels == 0 {
            return Ok(false);
        } else {
            if let Some(children) = &mut self.children {
                for child in children {
                    if x >= child.x && x < child.x+child.width as i32 && y >= child.y && y < child.y + child.height as i32 {
                        let e = child.insert(x, y);
                        if let Ok(true) = &e {
                            self.free_pixels -= 1;
                        }
                        return e;
                    }
                }
            } else {
                if self.width * self.height > (LEAF_SIZE*LEAF_SIZE) as u32 {
                    self.children = Some(vec![
                        CollisionTree::new(self.x, self.y, self.width / 2, self.height / 2),
                        CollisionTree::new(self.x + self.width as i32 / 2, self.y, self.width / 2, self.height / 2),
                        CollisionTree::new(self.x + self.width as i32 / 2, self.y + self.height as i32  / 2, self.width / 2, self.height / 2),
                        CollisionTree::new(self.x, self.y + self.height as i32 / 2, self.width / 2, self.width / 2),
                    ]);
                    if let Some(grid) = self.grid.take() {
                        for (i, v) in grid.iter().enumerate() {
                            if *v {
                                let lx = i as i32 % LEAF_SIZE as i32;
                                let ly = i as i32 / LEAF_SIZE as i32;
                                for child in self.children.as_mut().unwrap() {
                                    if x + lx >= child.x && x + lx < child.x+child.width as i32 && y  + ly >= child.y && y + ly < child.y + child.height as i32 {
                                        child.insert(x + lx, y + ly);
                                        break;
                                    }
                                }
                            }
                        }
                    }
                    for child in self.children.as_mut().unwrap().iter_mut() {
                        if x >= child.x && x < child.x+child.width as i32 && y >= child.y && y < child.y + child.height as i32 {
                            let e = child.insert(x, y);
                            if let Ok(true) = &e {
                                self.free_pixels -= 1;
                            }
                            return e;
                        }
                    }
                } else {
                    if self.grid.is_none() {
                        self.grid.replace([false; LEAF_SIZE*LEAF_SIZE]);
                    }
                    let i = ((x - self.x) + (y - self.y) * self.width as i32) as usize;
                    let p = &mut self.grid.as_mut().unwrap()[i];
                    if !*p {
                        *p = true;
                        self.free_pixels -= 1;
                        return Ok(true);
                    } else {
                        return Ok(false);
                    }
                }
            }
        }
        unreachable!();
    }

    fn add_sprite(&mut self, sprite: &Sprite) {
        for x in 0..SPRITE_WIDTH {
            for y in 0..SPRITE_WIDTH {
                let i = x + y * SPRITE_WIDTH;
                if sprite.collider[i] {
                    let rx = x as i32 * sprite.x_scale as i32 + sprite.loc.x as i32;
                    let ry = y as i32 * sprite.y_scale as i32 + sprite.loc.y as i32;
                    for dx in 0..sprite.x_scale as i32 {
                        for dy in 0..sprite.y_scale as i32 {
                            self.insert(rx + dx, ry + dy);
                        }
                    }
                }
            }
        }
    }

    fn clear_sprite(&mut self, sprite: Sprite) {
        for x in 0..SPRITE_WIDTH {
            for y in 0..SPRITE_WIDTH {
                let i = x + y * SPRITE_WIDTH;
                if sprite.collider[i] {
                    let rx = x as i32 * sprite.x_scale as i32 + sprite.loc.x as i32;
                    let ry = y as i32 * sprite.y_scale as i32 + sprite.loc.y as i32;
                    self.remove_rect(rx, ry, sprite.x_scale, sprite.y_scale);
                }
            }
        }
    }

    fn check_point(&self, x: i32, y: i32) -> bool {
        if x < self.x || x >= self.x+self.width as i32 || y < self.y || y >= self.y+self.height as i32 {
            return false;
        }
        if self.free_pixels == 0 {
            return true;
        }
        if let Some(grid) = &self.grid {
            let x = x - self.x;
            let y = y - self.y;
            let i = (x + y * self.width as i32) as usize;
            return grid[i];
        } else {
            if let Some(children) = &self.children {
                for child in children {
                    if child.check_point(x, y) {
                        return true;
                    }
                }
            }
        }
        false
    }

    fn remove_rect(&mut self, x: i32, y: i32, width: u32, height: u32) -> (bool, u32) {
        if x + width as i32 <= self.x || x > self.x + self.width as i32 || y + height as i32 <= self.y || y > self.y + self.height as i32 {
            return (false, 0);
        }
        if x <= self.x && x + width as i32 > self.x+self.width as i32 && y <= self.y && y + height as i32 > self.y+self.height as i32 {
            self.children.take();
            self.grid.take();
            return (true, self.width * self.height - self.free_pixels);
        }
        if let Some(grid) = &mut self.grid {
            let mut removed = 0;
            for x in self.x.max(x)..(self.x+self.width as i32).min(x+width as i32) {
                for y in self.y.max(y)..(self.y+self.height as i32).min(y+height as i32) {
                    let x = x - self.x;
                    let y = y - self.y;
                    let i = (x + y * self.width as i32) as usize;
                    if grid[i] {
                        self.free_pixels += 1;
                        removed += 1;
                        grid[i] = false;
                    }
                }
            }
            if self.free_pixels == self.width * self.height {
                return (true, removed);
            }
            return (false, removed);
        } else {
            let mut keep = false;
            let mut removed = 0;
            if let Some(children) = &mut self.children {
                for child in children {
                    let (empty, child_removed) = child.remove_rect(x, y, width, height);
                    removed += child_removed;
                    if !empty {
                        keep = true;
                    } else {
                    }
                }
            }
            if !keep {
                self.children.take();
            }
            self.free_pixels += removed;
            return (!keep, removed);
        }
        (false, 0)
    }

    fn check_rect(&self, x: i32, y: i32, width: u32, height: u32) -> bool {
        if x + width as i32 <= self.x || x > self.x + self.width as i32 || y + height as i32 <= self.y || y > self.y + self.height as i32 {
            return false;
        }
        if self.free_pixels == 0 {
            return true;
        }
        if self.free_pixels == self.width * self.height {
            return false;
        }
        if x <= self.x && x + width as i32 > self.x+self.width as i32 && y <= self.y && y + height as i32 > self.y+self.height as i32 {
            if self.free_pixels < self.width * self.height {
                return true;
            }
        }
        if let Some(grid) = &self.grid {
            for x in self.x.max(x)..(self.x+self.width as i32).min(x+width as i32) {
                for y in self.y.max(y)..(self.y+self.height as i32).min(y+height as i32) {
                    let x = x - self.x;
                    let y = y - self.y;
                    let i = (x + y * self.width as i32) as usize;
                    if grid[i] {
                        return true;
                    }
                }
            }
        } else {
            if let Some(children) = &self.children {
                for child in children {
                    if child.check_rect(x, y, width, height) {
                        return true;
                    }
                }
            }
        }
        false
    }
}

const TILE_SIZE: u32 = 128;

struct Scene {
    sprites: IndexMap<usize, Sprite>,
    sprite_cache: IndexMap<usize, Image>,
    potions: Vec<(usize, i32, i32)>,
    characters: Vec<usize>,
    particles: Vec<usize>,
    collision_map: CollisionTree,
    rubble_map: CollisionTree,
    next_id: usize,
    tile_cache: IndexMap<(i32, i32), Option<Image>>,
    foreground_map: CollisionTree,
    background_map: CollisionTree,
    foreground_tile_cache: IndexMap<(i32, i32), Option<Image>>,
    background_tile_cache: IndexMap<(i32, i32), Option<Image>>,
}

fn to_scale(x: i32, y: i32, x_scale: u32, y_scale: u32) -> (i32, i32) {
    let x = x / x_scale as i32;
    let y = y / y_scale as i32;
    (x,y)
}

fn from_scale(x: i32, y: i32, x_scale: u32, y_scale: u32) -> (i32, i32) {
    let x = x * x_scale as i32;
    let y = y * y_scale as i32;
    (x,y)
}

impl Scene {
    fn new() -> Self {
        let world_min = -10000;
        let world_width = 40000;
        let mut tile_cache = IndexMap::new();
        let mut foreground_tile_cache = IndexMap::new();
        let mut background_tile_cache = IndexMap::new();
        for x in world_min/TILE_SIZE as i32..(world_min+world_width)/TILE_SIZE as i32 {
            for y in world_min/TILE_SIZE as i32..(world_min+world_width)/TILE_SIZE as i32 {
                tile_cache.insert((x, y), None);
                foreground_tile_cache.insert((x, y), None);
                background_tile_cache.insert((x, y), None);
            }
        }
        Self {
            sprites: IndexMap::new(),
            sprite_cache: IndexMap::new(),
            potions: vec![],
            characters: vec![],
            particles: vec![],
            collision_map: CollisionTree::new(world_min, world_min, world_width as u32, world_width as u32),
            rubble_map: CollisionTree::new(world_min, world_min, world_width as u32, world_width as u32),
            next_id: 0,
            tile_cache,
            foreground_map: CollisionTree::new(world_min, world_min, world_width as u32, world_width as u32),
            background_map: CollisionTree::new(world_min, world_min, world_width as u32, world_width as u32),
            foreground_tile_cache,
            background_tile_cache,
        }
    }

    fn add_sprite(&mut self, sprite: Sprite) -> usize {
        let id = self.next_id;
        self.next_id += 1;
        self.sprites.insert(id, sprite);
        id
    }

    fn add_potion(&mut self, sprite: Sprite, x_delta: i32, y_delta: i32) -> usize {
        let id = self.add_sprite(sprite);
        self.potions.push((id, x_delta, y_delta));
        id
    }

    fn add_particle(&mut self, sprite: Sprite) -> usize {
        let id = self.add_sprite(sprite);
        self.particles.push(id);
        id
    }

    fn add_character(&mut self, sprite: Sprite) -> usize {
        let id = self.add_sprite(sprite);
        self.characters.push(id);
        id
    }

    fn add_terrain(&mut self, sprite: &Sprite) {
        self.collision_map.add_sprite(sprite);
        for x in sprite.loc.x as i32..sprite.loc.x as i32 + SPRITE_WIDTH as i32 * sprite.x_scale as i32 {
            for y in sprite.loc.y as i32..sprite.loc.y as i32 + SPRITE_WIDTH as i32 * sprite.y_scale as i32 {
                self.tile_cache.remove(&(x / TILE_SIZE as i32, y / TILE_SIZE as i32));
            }
        }
    }

    fn clear_terrain(&mut self, sprite: Sprite) {
        for x in sprite.loc.x as i32..sprite.loc.x as i32 + SPRITE_WIDTH as i32 * sprite.x_scale as i32 {
            for y in sprite.loc.y as i32..sprite.loc.y as i32 + SPRITE_WIDTH as i32 * sprite.y_scale as i32 {
                self.tile_cache.remove(&(x / TILE_SIZE as i32, y / TILE_SIZE as i32));
            }
        }
        self.collision_map.clear_sprite(sprite);
    }

    fn add_foreground(&mut self, sprite: &Sprite) {
        self.foreground_map.add_sprite(sprite);
        for x in sprite.loc.x as i32..sprite.loc.x as i32 + SPRITE_WIDTH as i32 * sprite.x_scale as i32 {
            for y in sprite.loc.y as i32..sprite.loc.y as i32 + SPRITE_WIDTH as i32 * sprite.y_scale as i32 {
                self.foreground_tile_cache.remove(&(x / TILE_SIZE as i32, y / TILE_SIZE as i32));
            }
        }
    }

    fn add_background(&mut self, sprite: &Sprite) {
        self.background_map.add_sprite(sprite);
        for x in sprite.loc.x as i32..sprite.loc.x as i32 + SPRITE_WIDTH as i32 * sprite.x_scale as i32 {
            for y in sprite.loc.y as i32..sprite.loc.y as i32 + SPRITE_WIDTH as i32 * sprite.y_scale as i32 {
                self.background_tile_cache.remove(&(x / TILE_SIZE as i32, y / TILE_SIZE as i32));
            }
        }
    }

    fn step_physics(&mut self) {
        for sprite in self.sprites.values_mut() {
            if sprite.is_player {
                // Collision resolution
                let mut x_dir = 0;
                let mut y_dir = 0;
                for dx in 0..SPRITE_WIDTH {
                    for dy in 0..SPRITE_WIDTH {
                        let i = dx + dy*SPRITE_WIDTH;
                        if sprite.collider[i] {
                            let x = sprite.loc.x as i32 + dx as i32 * sprite.x_scale as i32;
                            let y = sprite.loc.y as i32 + dy as i32 * sprite.y_scale as i32;
                            if self.rubble_map.check_rect(x, y, sprite.x_scale, sprite.y_scale) {
                            } else if self.collision_map.check_rect(x, y, sprite.x_scale, sprite.y_scale) {
                                if dx <= SPRITE_WIDTH/2 {
                                    x_dir += 1;
                                } else {
                                    x_dir -= 1;
                                }
                                if dy <= SPRITE_WIDTH/2 {
                                    y_dir += 1;
                                } else {
                                    y_dir -= 1;
                                }
                            }
                        }
                    }
                }
                sprite.loc.x += (x_dir.max(-1).min(1) * sprite.x_scale as i32) as f32;
                sprite.loc.y += (y_dir.max(-1).min(1) * sprite.y_scale as i32) as f32;
            }

            sprite.velocity.y += 3.4 / FPS;
            let mut blocked_x = false;
            let mut blocked_y = false;
            let mut blocked_by_ground = false;
            let mut in_rubble = false;
            let falling = sprite.velocity.y > 0.0;
            for (mut vx, mut vy) in vec![(0, (sprite.velocity.y * sprite.y_scale as f32) as i32), ((sprite.velocity.x * sprite.x_scale as f32) as i32, 0)] {
                {
                    let mut loc_x = sprite.loc.x;
                    let mut loc_y = sprite.loc.y;

                    let step_x = (sprite.x_scale as f32 / 8.0).min(1.0).min(sprite.velocity.x.abs()).max(1.0).copysign(sprite.velocity.x);
                    let step_y = (sprite.y_scale as f32 / 8.0).min(1.0).min(sprite.velocity.y.abs()).max(1.0).copysign(sprite.velocity.y);

                    'outer: while vy.abs() >= 1 || vx.abs() >= 1 {
                        if vy.abs() >= 1 {
                            loc_y += step_y;
                        } else {
                            loc_x += step_x;
                        }
                        for dx in 0..SPRITE_WIDTH {
                            for dy in 0..SPRITE_WIDTH {
                                let i = dx + dy*SPRITE_WIDTH;
                                if sprite.collider[i] {
                                    let x = loc_x as i32 + dx as i32 * sprite.x_scale as i32;
                                    let y = loc_y as i32 + dy as i32 * sprite.y_scale as i32;
                                    if self.rubble_map.check_rect(x, y, sprite.x_scale, sprite.y_scale) {
                                        in_rubble = true;
                                    } else if self.collision_map.check_rect(x, y, sprite.x_scale, sprite.y_scale) {
                                        if vx.abs() >= 1 {
                                            blocked_x = true;
                                        } else {
                                            blocked_y = true;
                                        }
                                        break 'outer;
                                    }
                                }
                            }
                        }
                        if vy.abs() >= 1 {
                            vy -= step_y as i32;
                        } else {
                            vx -= step_x as i32;
                        }
                        sprite.loc.x = loc_x;
                        sprite.loc.y = loc_y;
                    }
                }
            }
            if sprite.is_player && !in_rubble {
                self.rubble_map.clear();
            }
            if !blocked_y {
                if sprite.velocity.y.abs() >= 1.0 {
                    sprite.ground_contact = false;
                }
            } else  {
                if falling {
                    sprite.ground_contact = true;
                    sprite.jumping = false;
                }
                sprite.velocity.y = 0.0;
            }
            if sprite.ground_contact {
                if sprite.velocity.x >= 0.0 {
                    sprite.velocity.x = (sprite.velocity.x - 1.0/FPS).max(0.0);
                } else {
                    sprite.velocity.x = (sprite.velocity.x + 1.0/FPS).min(0.0);
                }
            }
            if sprite.velocity.x.abs() > 1.0 || sprite.velocity.y.abs() > 1.0 {
                sprite.sleep_timer = 0.0;
            } else {
                sprite.sleep_timer += 1.0/FPS;
            }
        }

        let mut to_remove = IndexSet::new();
        for particle_id in &self.particles {
            let sprite = &self.sprites[particle_id];
            if sprite.loc.y > 30000.0 {
                to_remove.insert(*particle_id);
                self.sprite_cache.remove(particle_id);
                continue;
            }
            if sprite.ground_contact && sprite.sleep_timer > 0.5 {
                to_remove.insert(*particle_id);
                for x in 0..SPRITE_WIDTH {
                    for y in 0..SPRITE_WIDTH {
                        let i = x + y*SPRITE_WIDTH;
                        if sprite.collider[i as usize] {
                            for dx in 0..sprite.x_scale {
                                for dy in 0..sprite.y_scale {
                                    let x = sprite.loc.x as i32 + x as i32 * sprite.x_scale as i32 + dx as i32;
                                    let y = sprite.loc.y as i32 + y as i32 * sprite.y_scale as i32 + dy as i32;
                                    self.collision_map.insert(x ,y);
                                    self.rubble_map.insert(x, y);
                                    self.tile_cache.remove(&(x/ TILE_SIZE as i32, y / TILE_SIZE as i32));
                                }
                            }
                        }
                    }
                }
            }
        }
        self.particles.retain(|pid| !to_remove.contains(pid));
        self.sprites.retain(|pid, _| !to_remove.contains(pid));

        let mut drinkers = vec![];
        let mut consumed = IndexSet::new();
        for character_id in &self.characters {
            let character = &self.sprites[character_id];
            for (potion_id, x_delta, y_delta) in &self.potions {
                if consumed.contains(potion_id) {
                    continue;
                }
                let potion = &self.sprites[potion_id];
                if character.overlap(potion) {
                    consumed.insert(*potion_id);
                    drinkers.push((*character_id, *x_delta, *y_delta));
                }
            }
        }
        for potion_id in consumed {
            self.potions.retain(|(id, _, _)| *id != potion_id);
            self.sprites.remove(&potion_id);
            self.sprite_cache.remove(&potion_id);
        }
        let mut new_sprites = vec![];
        for (sprite_id, x_delta, y_delta) in drinkers {
            let sprite = self.sprites.get_mut(&sprite_id).unwrap();
            if sprite.scale_timer.is_none() {
                sprite.scale_timer = Some(SCALE_CHANGE_TIMEOUT);
            }
            sprite.pending_x_scale += x_delta;
            sprite.pending_y_scale += y_delta;
        }

        for character_id in self.characters.clone() {
            let sprite = self.sprites.get_mut(&character_id).unwrap();
            if let Some(time) = sprite.scale_timer.as_mut() {
                *time -= 1.0/FPS;
                if *time > 0.0 {
                    continue
                }
                sprite.scale_timer.take();
                let x_delta = sprite.pending_x_scale;
                let y_delta = sprite.pending_y_scale;
                sprite.pending_x_scale = 0;
                sprite.pending_y_scale = 0;
                if x_delta == 0 && y_delta == 0 {
                    continue
                }
                let initial_width = SPRITE_WIDTH as u32 * sprite.x_scale;
                let initial_height = SPRITE_WIDTH as u32 * sprite.y_scale;
                sprite.x_scale = (sprite.x_scale as i32 + x_delta).max(1) as u32;
                sprite.y_scale = (sprite.y_scale as i32 + y_delta).max(1) as u32;
                sprite.loc.x -= (SPRITE_WIDTH as f32 * sprite.x_scale as f32 - initial_width as f32)/2.0;
                sprite.loc.y -= SPRITE_WIDTH as f32 * sprite.y_scale as f32 - initial_height as f32;
                //FIXME: Why is this offset necessary?
                sprite.loc.y -= 8.0;
                if x_delta > 0 || y_delta > 0 {
                    let cx = sprite.loc.x + SPRITE_WIDTH as f32 / 2.0;
                    let cy = sprite.loc.y + SPRITE_WIDTH as f32 / 2.0;
                    for dx in 0..SPRITE_WIDTH {
                        for dy in -1..SPRITE_WIDTH as i32-1 {
                            if true {
                                let x = sprite.loc.x as i32 + dx as i32 * sprite.x_scale as i32;
                                let y = sprite.loc.y as i32 + dy as i32 * sprite.y_scale as i32;
                                if Vector::new(cx, cy).distance(Vector::new(x as f32, y as f32)) < SPRITE_WIDTH as f32 * sprite.x_scale.max(sprite.y_scale) as f32 {
                                    if self.foreground_map.remove_rect(x, y, sprite.x_scale, sprite.y_scale).1 > 0 {
                                        for xx in 0..sprite.x_scale {
                                            for yy in 0..sprite.y_scale {
                                                let cx = sprite.loc.x as i32 + dx as i32 * sprite.x_scale as i32 + xx as i32;
                                                let cy = sprite.loc.y as i32 + dy as i32 * sprite.y_scale as i32 + yy as i32;
                                                self.foreground_tile_cache.remove(&(cx / TILE_SIZE as i32, cy / TILE_SIZE as i32));
                                            }
                                        }
                                    }
                                    if self.collision_map.remove_rect(x, y, sprite.x_scale, sprite.y_scale).1 > 0 {
                                        let mut collider = [false; SPRITE_WIDTH*SPRITE_WIDTH];
                                        collider[0] = true;
                                        let mut new_sprite = Sprite::from_collider(collider, x as f32, y as f32, sprite.x_scale, sprite.y_scale, Color::WHITE);
                                        let a = (cy-y as f32).atan2(cx - x as f32) + std::f32::consts::FRAC_PI_4;
                                        new_sprite.velocity = Vector::new(a.cos() * 100.0/FPS, a.sin() * 100.0/FPS);
                                        new_sprites.push(new_sprite);
                                        for xx in 0..sprite.x_scale {
                                            for yy in 0..sprite.y_scale {
                                                let cx = sprite.loc.x as i32 + dx as i32 * sprite.x_scale as i32 + xx as i32;
                                                let cy = sprite.loc.y as i32 + dy as i32 * sprite.y_scale as i32 + yy as i32;
                                                self.tile_cache.remove(&(cx / TILE_SIZE as i32, cy / TILE_SIZE as i32));
                                            }
                                        }

                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        if !new_sprites.is_empty() {
            //self.rubble_map.clear();
        }
        for sprite in new_sprites {
            self.add_particle(sprite);
        }
    }

    fn draw(&mut self, gfx: &mut Graphics, x: i32, y: i32, width: u32, height: u32, scale: f32) {
        let pwidth = width;
        let pheight = height;
        let width = (width as f32 * scale) as u32;
        let height = (height as f32 * scale) as u32;
        let x = x - width as i32 / 2;
        let y = y - height as i32 / 2;

        for xx in x/TILE_SIZE as i32 - 1..(x+width as i32)/TILE_SIZE as i32 + 1 {
            for yy in y/TILE_SIZE as i32 - 1..(y+height as i32) / TILE_SIZE as i32 + 1 {
                if !self.background_tile_cache.contains_key(&(xx, yy)) {
                    let mut tile = vec![0; (TILE_SIZE*TILE_SIZE*4) as usize];
                    for dx in 0..TILE_SIZE {
                        for dy in 0..TILE_SIZE {
                            if self.background_map.check_point(xx * TILE_SIZE as i32+ dx as i32, yy * TILE_SIZE as i32 + dy as i32) {
                                let i = (dx + dy * TILE_SIZE) as usize * 4;
                                tile[i] = (BACKGROUND_COLOR.r * 255.0).clamp(0.0, 255.0) as u8;
                                tile[i+1] = (BACKGROUND_COLOR.g * 255.0).clamp(0.0, 255.0) as u8;
                                tile[i+2] = (BACKGROUND_COLOR.b * 255.0).clamp(0.0, 255.0) as u8;
                                tile[i+3] = 255;
                            }
                        }
                    }
                    let mut tile = Image::from_raw(
                        gfx,
                        Some(&tile),
                        TILE_SIZE,
                        TILE_SIZE,
                        PixelFormat::RGBA,
                    ).unwrap();
                    tile.set_magnification(golem::TextureFilter::Nearest).unwrap();
                    self.background_tile_cache.insert((xx, yy), Some(tile));
                }
                if let Some(Some(tile)) = self.background_tile_cache.get(&(xx, yy)) {
                    let region = Rectangle::new(Vector::new((xx*TILE_SIZE as i32 - x) as f32 / scale, (yy*TILE_SIZE as i32 - y) as f32 / scale), Vector::new(TILE_SIZE as f32 / scale, TILE_SIZE as f32 / scale));
                    gfx.draw_image(tile, region);
                }
            }
        }

        for (sprite_id, sprite) in &self.sprites {
            let sx = sprite.loc.x - x as f32;
            let sy = sprite.loc.y - y as f32;
            let w = (SPRITE_WIDTH as u32 * sprite.x_scale) as f32;
            let h = (SPRITE_WIDTH as u32 * sprite.y_scale) as f32;
            if sx > -w && sx < width as f32 && sy > -h && sy < height as f32 {
                if !self.sprite_cache.contains_key(sprite_id) {
                    self.sprite_cache.insert(*sprite_id, sprite.image(gfx));
                }
                let sprite_image = &self.sprite_cache[sprite_id];
                let region = Rectangle::new(Vector::new((sprite.loc.x as i32 - x) as f32 /scale, (sprite.loc.y as i32 - y) as f32/scale), Vector::new(w/scale, h/scale));
                gfx.draw_image(sprite_image, region);
                if let Some(t) = sprite.scale_timer {
                    let red_shift: u8 = ((t * (10.0 + ((SCALE_CHANGE_TIMEOUT - t) / SCALE_CHANGE_TIMEOUT) * 20.0).sin() + 1.0) * 255.0) as u8;
                    let mut pixels = [0; SPRITE_WIDTH*SPRITE_WIDTH*4];
                    for x in 0..SPRITE_WIDTH {
                        for y in 0..SPRITE_WIDTH {
                            let i = (x + y*SPRITE_WIDTH as usize);
                            if sprite.collider[i] {
                                pixels[i * 4] = red_shift;
                                pixels[i * 4 + 1] = 0xff;
                                pixels[i * 4 + 1] = 0xff;
                                pixels[i * 4+ 3] = 100;
                            }
                        }
                    }
                    let mut overlay = Image::from_raw(
                        gfx,
                        Some(&pixels),
                        SPRITE_WIDTH as u32,
                        SPRITE_WIDTH as u32,
                        PixelFormat::RGBA,
                    ).unwrap();
                    overlay.set_magnification(golem::TextureFilter::Nearest).unwrap();
                    gfx.draw_image(&overlay, region);
                }
            }
        }


        for xx in x/TILE_SIZE as i32 - 1..(x+width as i32)/TILE_SIZE as i32 + 1 {
            for yy in y/TILE_SIZE as i32 - 1..(y+height as i32) / TILE_SIZE as i32 + 1 {
                if !self.tile_cache.contains_key(&(xx, yy)) {
                    let mut tile = vec![0; (TILE_SIZE*TILE_SIZE*4) as usize];
                    for dx in 0..TILE_SIZE {
                        for dy in 0..TILE_SIZE {
                            if self.collision_map.check_point(xx * TILE_SIZE as i32+ dx as i32, yy * TILE_SIZE as i32 + dy as i32) {
                                let i = (dx + dy * TILE_SIZE) as usize * 4;
                                tile[i] = (TERRAIN_COLOR.r * 255.0).clamp(0.0, 255.0) as u8;
                                tile[i+1] = (TERRAIN_COLOR.g * 255.0).clamp(0.0, 255.0) as u8;
                                tile[i+2] = (TERRAIN_COLOR.b * 255.0).clamp(0.0, 255.0) as u8;
                                tile[i+3] = 0xff;
                            }
                        }
                    }
                    let mut tile = Image::from_raw(
                        gfx,
                        Some(&tile),
                        TILE_SIZE,
                        TILE_SIZE,
                        PixelFormat::RGBA,
                    ).unwrap();
                    tile.set_magnification(golem::TextureFilter::Nearest).unwrap();
                    self.tile_cache.insert((xx, yy), Some(tile));
                }
                if let Some(Some(tile)) = self.tile_cache.get(&(xx, yy)) {
                    let region = Rectangle::new(Vector::new((xx*TILE_SIZE as i32 - x) as f32 / scale, (yy*TILE_SIZE as i32 - y) as f32 / scale), Vector::new(TILE_SIZE as f32 / scale, TILE_SIZE as f32 / scale));
                    gfx.draw_image(tile, region);
                }
            }
        }

        for xx in x/TILE_SIZE as i32 - 1..(x+width as i32)/TILE_SIZE as i32 + 1 {
            for yy in y/TILE_SIZE as i32 - 1..(y+height as i32) / TILE_SIZE as i32 + 1 {
                if !self.foreground_tile_cache.contains_key(&(xx, yy)) {
                    let mut tile = vec![0; (TILE_SIZE*TILE_SIZE*4) as usize];
                    for dx in 0..TILE_SIZE {
                        for dy in 0..TILE_SIZE {
                            if self.foreground_map.check_point(xx * TILE_SIZE as i32+ dx as i32, yy * TILE_SIZE as i32 + dy as i32) {
                                let i = (dx + dy * TILE_SIZE) as usize * 4;
                                tile[i] = (FOREGROUND_COLOR.r * 255.0).clamp(0.0, 255.0) as u8;
                                tile[i+1] = (FOREGROUND_COLOR.g * 255.0).clamp(0.0, 255.0) as u8;
                                tile[i+2] = (FOREGROUND_COLOR.b * 255.0).clamp(0.0, 255.0) as u8;
                                tile[i+3] = 255;
                            }
                        }
                    }
                    let mut tile = Image::from_raw(
                        gfx,
                        Some(&tile),
                        TILE_SIZE,
                        TILE_SIZE,
                        PixelFormat::RGBA,
                    ).unwrap();
                    tile.set_magnification(golem::TextureFilter::Nearest).unwrap();
                    self.foreground_tile_cache.insert((xx, yy), Some(tile));
                }
                if let Some(Some(tile)) = self.foreground_tile_cache.get(&(xx, yy)) {
                    let region = Rectangle::new(Vector::new((xx*TILE_SIZE as i32 - x) as f32 / scale, (yy*TILE_SIZE as i32 - y) as f32 / scale), Vector::new(TILE_SIZE as f32 / scale, TILE_SIZE as f32 / scale));
                    gfx.draw_image(tile, region);
                }
            }
        }

    }
}

async fn app(window: Window, mut gfx: Graphics, mut input: Input) -> Result<()> {
    let sprites = image::load(std::io::Cursor::new(SPRITES), image::ImageFormat::Png).unwrap();
    let map_data = quicksilver::load_file("map.tmx").await.expect("The file was not found!");
    let map = tiled::parse(&*map_data).unwrap();
    let mut scene = Scene::new();
    let mut player_id = None;
    let mut negative_terrain = vec![];
    let mut terrain_locations = IndexSet::new();
    for group in &map.object_groups {
        if !group.visible {
            continue
        }
        for object in &group.objects {
            let x_scale = (object.width / 16.0) as u32;
            let y_scale = (object.height / 16.0) as u32;
            assert_eq!(x_scale as f32 * 16.0, object.width, "badly scaled sprite {} in {}", object.id, group.name);
            assert_eq!(y_scale as f32 * 16.0, object.height);
            let ty = (object.gid - 1) / 48;
            let tx = (object.gid - 1) - ty as u32 * 48;


            if group.name == "player" || group.name == "test_player" {
                player_id = Some(scene.add_character(Sprite::new(&sprites, tx as usize, ty as usize, object.x, object.y - object.height, x_scale, y_scale, Color::BLUE)));
            } else if group.name == "objects" {
                let x_delta = if let Some(v) = object.properties.get("x_delta") {
                    match v {
                        tiled::PropertyValue::IntValue(v) => *v,
                        _ => 1,
                    }
                } else if let Some(v) = object.properties.get("delta") {
                    match v {
                        tiled::PropertyValue::IntValue(v) => *v,
                        _ => 1,
                    }
                } else {
                    1
                };
                let y_delta = if let Some(v) = object.properties.get("y_delta") {
                    match v {
                        tiled::PropertyValue::IntValue(v) => *v,
                        _ => 1,
                    }
                } else if let Some(v) = object.properties.get("delta") {
                    match v {
                        tiled::PropertyValue::IntValue(v) => *v,
                        _ => 1,
                    }
                } else {
                    1
                };
                scene.add_potion(Sprite::new(&sprites, tx as usize, ty as usize, object.x, object.y - object.height, x_scale, y_scale, if x_delta > 0 || y_delta > 0 { Color::RED } else { Color::BLUE }), x_delta, y_delta);
            } else if group.name.starts_with("terrain") {
                terrain_locations.insert((object.x as i32 / TILE_SIZE as i32, (object.y - object.height) as i32 / TILE_SIZE as i32));
                scene.add_terrain(&Sprite::new(&sprites, tx as usize, ty as usize, object.x, object.y - object.height, x_scale, y_scale, Color::RED));
            } else if group.name.starts_with("negative-terrain") {
                terrain_locations.insert((object.x as i32 / TILE_SIZE as i32, (object.y - object.height) as i32 / TILE_SIZE as i32));
                negative_terrain.push(Sprite::new(&sprites, tx as usize, ty as usize, object.x, object.y - object.height, x_scale, y_scale, Color::RED));
            } else if group.name == "background" {
                terrain_locations.insert((object.x as i32 / TILE_SIZE as i32, (object.y - object.height) as i32 / TILE_SIZE as i32));
                scene.add_background(&Sprite::new(&sprites, tx as usize, ty as usize, object.x, object.y - object.height, x_scale, y_scale, Color::RED));
            } else if group.name == "foreground" {
                terrain_locations.insert((object.x as i32 / TILE_SIZE as i32, (object.y - object.height) as i32 / TILE_SIZE as i32));
                scene.add_foreground(&Sprite::new(&sprites, tx as usize, ty as usize, object.x, object.y - object.height, x_scale, y_scale, Color::RED));
            }
        }
    }
    for terrain in negative_terrain {
        scene.clear_terrain(terrain);
    }

    for (sprite_id, sprite) in &scene.sprites {
        scene.sprite_cache.insert(*sprite_id, sprite.image(&gfx));
    }

    let mut terrain_locations: Vec<_> = terrain_locations.into_iter().map(|(x,y)| Vector::new(x as f32, y as f32)).collect();

    let player_id = player_id.unwrap();


    let mut camera = Vector::new(0.0, 0.0);
    let mut camera_scale = 8.0;
    {
        let player = &mut scene.sprites[player_id];
        player.is_player = true;
        camera.x = player.loc.x;
        camera.y = player.loc.y;
        camera_scale = player.x_scale.max(player.y_scale) as f32;
        //player.collider[2 + 4 * SPRITE_WIDTH] = false;
        //player.collider[SPRITE_WIDTH-3 + 4 * SPRITE_WIDTH] = false;
    }

    let mut update_timer = Timer::time_per_second(FPS);
    let mut draw_timer = Timer::time_per_second(FPS);
    let mut moving_left = false;
    let mut moving_right = false;
    loop {
        {
            if !terrain_locations.is_empty() {
                let player_loc = scene.sprites[player_id].loc / TILE_SIZE as f32;
                terrain_locations.sort_by_key(|t| (player_loc.distance(*t) * 10000.0) as i32);
                let loc = terrain_locations.remove(0);
                let x = loc.x as i32 / TILE_SIZE as i32;
                let y = loc.y as i32 / TILE_SIZE as i32;
                for (cache, map, color) in &mut [
                     (&mut scene.foreground_tile_cache, &scene.foreground_map, FOREGROUND_COLOR),
                     (&mut scene.background_tile_cache, &scene.background_map, BACKGROUND_COLOR),
                     (&mut scene.tile_cache, &scene.collision_map, TERRAIN_COLOR),
                  ] {
                    if !cache.contains_key(&(x, y)) {
                        let mut tile = vec![0; (TILE_SIZE*TILE_SIZE*4) as usize];
                        for dx in 0..TILE_SIZE {
                            for dy in 0..TILE_SIZE {
                                if map.check_point(x * TILE_SIZE as i32+ dx as i32, y * TILE_SIZE as i32 + dy as i32) {
                                    let i = (dx + dy * TILE_SIZE) as usize * 4;
                                    tile[i] = (color.r * 255.0).clamp(0.0, 255.0) as u8;
                                    tile[i+1] = (color.g * 255.0).clamp(0.0, 255.0) as u8;
                                    tile[i+2] = (color.b * 255.0).clamp(0.0, 255.0) as u8;
                                    tile[i+3] = 255;
                                }
                            }
                        }
                        let mut tile = Image::from_raw(
                            &gfx,
                            Some(&tile),
                            TILE_SIZE,
                            TILE_SIZE,
                            PixelFormat::RGBA,
                        ).unwrap();
                        tile.set_magnification(golem::TextureFilter::Nearest).unwrap();
                        cache.insert((x, y), Some(tile));
                    }
                  }
            }
        }
        while let Some(e) = input.next_event().await {
            let player = scene.sprites.get_mut(&player_id).unwrap();
            match e {
                Event::KeyboardInput(e) => {
                    match e.key() {
                        Key::Right => {
                            if e.is_down() {
                                moving_right = true;
                            } else {
                                moving_right = false;
                            }
                        },
                        Key::Left => {
                            if e.is_down() {
                                moving_left = true;
                            } else {
                                moving_left = false;
                            }
                        },
                        Key::Up => {
                            if e.is_down() {
                                if player.ground_contact {
                                    player.jumping = true;
                                    player.velocity.y = -80.0 / FPS;
                                }
                            } else {
                                if !player.ground_contact && player.jumping {
                                    player.velocity.y = player.velocity.y.max(-2.0);
                                } 

                            }
                        },
                        Key::A => {
                            if e.is_down() {
                                player.x_scale += 1;
                                player.y_scale += 1;
                            }
                        },
                        Key::O => {
                            if e.is_down() {
                                player.x_scale -= 1;
                                player.y_scale -= 1;
                            }
                        },
                        _ => (),
                    }
                },
                _ => (),
            }
        }

        {
            let player = scene.sprites.get_mut(&player_id).unwrap();
            let vx = if input.key_down(Key::LShift) && player.ground_contact {
                130.0
            } else {
                60.0
            };
            if moving_right {
                player.velocity.x = vx / FPS;
            } else if moving_left {
                player.velocity.x = -vx / FPS;
            } else {
                player.velocity.x = 0.0;
            }
        }
        while update_timer.tick() {
        //if update_timer.exhaust().is_some() {
            scene.step_physics();
        }
        if draw_timer.exhaust().is_some() {
            let player = scene.sprites.get_mut(&player_id).unwrap();
            camera.x = camera.x * 0.9 + (player.loc.x)*0.1;
            camera.y = camera.y * 0.9 + (player.loc.y) *0.1;
            camera_scale = camera_scale * 0.9 + player.x_scale.max(player.y_scale).min(30) as f32 * 0.1;
            gfx.clear(Color::BLACK);
            let scale = if camera_scale > 8.0 {
                (camera_scale / 8.0) as f32
            } else {
                1.0 / (8.0 / camera_scale) as f32
            };
            scene.draw(&mut gfx, camera.x as i32, camera.y as i32, 1920, 1080, scale);
            gfx.present(&window)?;
            //std::process::exit(0);
        }
    }
}
