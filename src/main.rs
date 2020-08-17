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
const SCALE_CHANGE_TIMEOUT: f32 = 2.0;

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
    collider: [bool; SPRITE_WIDTH*SPRITE_WIDTH],
    loc: Vector,
    scale: u32,
    velocity: Vector,
    ground_contact: bool,
    jumping: bool,
    vy_slop: f32,
    color: Color,
    scale_timer: Option<f32>,
    pending_scale: i32,
    sleep_timer: f32,
}

impl Sprite {
    fn new(src: &image::DynamicImage, x: usize, y: usize, xx: f32, yy: f32, scale: u32, color: Color) -> Self {
        let collider = extract_sprite(src, x, y);
        Sprite::from_collider(collider, xx, yy, scale, color)
    }

    fn from_collider(collider: [bool; SPRITE_WIDTH*SPRITE_WIDTH], xx: f32, yy: f32, scale: u32, color: Color) -> Self {
        Self {
            collider,
            loc: Vector::new(xx as f32, yy as f32),
            scale,
            velocity: Vector::new(0.0, 0.0),
            ground_contact: false,
            jumping: false,
            vy_slop: 0.0,
            color,
            scale_timer: None,
            pending_scale: 0,
            sleep_timer: 0.0,
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

const LEAF_SIZE: usize = 16;
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
                    let rx = x as i32 * sprite.scale as i32 + sprite.loc.x as i32;
                    let ry = y as i32 * sprite.scale as i32 + sprite.loc.y as i32;
                    for dx in 0..sprite.scale as i32 {
                        for dy in 0..sprite.scale as i32 {
                            self.insert(rx + dx, ry + dy);
                        }
                    }
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

const TILE_SIZE: u32 = 100;

struct Scene {
    sprites: IndexMap<usize, Sprite>,
    potions: Vec<(usize, i32)>,
    characters: Vec<usize>,
    particles: Vec<usize>,
    collision_map: CollisionTree,
    next_id: usize,
    tile_cache: IndexMap<(i32, i32), Image>,
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
            sprites: IndexMap::new(),
            potions: vec![],
            characters: vec![],
            particles: vec![],
            collision_map: CollisionTree::new(0, 0, 2000, 1000),
            next_id: 0,
            tile_cache: IndexMap::new(),
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
        let x = sprite.loc.x as i32 / sprite.scale as i32;
        let y = sprite.loc.y as i32 / sprite.scale as i32;
        self.collision_map.add_sprite(sprite);
    }

    fn step_physics(&mut self) {
        for sprite in self.sprites.values_mut() {
            sprite.velocity.y += 3.4 / FPS;
            let mut blocked_x = false;
            let mut blocked_y = false;
            let mut blocked_by_ground = false;
            for (mut vx, mut vy) in vec![(0, (sprite.velocity.y * sprite.scale as f32) as i32), ((sprite.velocity.x * sprite.scale as f32) as i32, 0)] {
                if vy != 0 && sprite.velocity.y < 0.0 {
                    sprite.loc.y += vy as f32;
                } else {
                    let mut loc_x = sprite.loc.x;
                    let mut loc_y = sprite.loc.y;

                    'outer: while vy.abs() >= 1 || vx.abs() >= 1 {
                        if vy.abs() >= 1 {
                            loc_y += 1.0f32.copysign(sprite.velocity.y);
                            vy -= vy.signum();
                        } else {
                            loc_x += 1.0f32.copysign(sprite.velocity.x);
                            vx -= vx.signum();
                        }
                        for dx in 0..SPRITE_WIDTH {
                            for dy in 0..SPRITE_WIDTH {
                                let i = dx + dy*SPRITE_WIDTH;
                                if sprite.collider[i] {
                                    let x = loc_x as i32 + dx as i32 * sprite.scale as i32;
                                    let y = loc_y as i32 + dy as i32 * sprite.scale as i32;
                                    if self.collision_map.check_rect(x, y, sprite.scale, sprite.scale) {
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
                        sprite.loc.x = loc_x;
                        sprite.loc.y = loc_y;
                    }
                }
            }
            if !blocked_y {
                if sprite.velocity.y.abs() >= 1.0 {
                    sprite.ground_contact = false;
                }
            } else  {
                if sprite.velocity.y > 0.0 {
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
            if sprite.velocity.x != 0.0 || sprite.velocity.y != 0.0 {
                sprite.sleep_timer = 0.0;
            } else {
                sprite.sleep_timer += 1.0/FPS;
            }
        }

        let mut to_remove = IndexSet::new();
        for particle_id in &self.particles {
            let sprite = &self.sprites[particle_id];
            if sprite.ground_contact && sprite.sleep_timer > 0.5 {
                to_remove.insert(*particle_id);
                for x in 0..SPRITE_WIDTH {
                    for y in 0..SPRITE_WIDTH {
                        let i = x + y*SPRITE_WIDTH;
                        if sprite.collider[i as usize] {
                            for dx in 0..sprite.scale {
                                for dy in 0..sprite.scale {
                                    let x = sprite.loc.x as i32 + x as i32 * sprite.scale as i32 + dx as i32;
                                    let y = sprite.loc.y as i32 + y as i32 * sprite.scale as i32 + dy as i32;
                                    self.collision_map.insert(x ,y);
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
            for (potion_id, delta) in &self.potions {
                if consumed.contains(potion_id) {
                    continue;
                }
                let potion = &self.sprites[potion_id];
                if character.overlap(potion) {
                    consumed.insert(*potion_id);
                    drinkers.push((*character_id, *delta));
                }
            }
        }
        for potion_id in consumed {
            self.potions.retain(|(id, _)| *id != potion_id);
            self.sprites.remove(&potion_id);
        }
        let mut new_sprites = vec![];
        for (sprite_id, delta) in drinkers {
            let sprite = self.sprites.get_mut(&sprite_id).unwrap();
            if sprite.scale_timer.is_none() {
                sprite.scale_timer = Some(SCALE_CHANGE_TIMEOUT);
            }
            sprite.pending_scale += delta;
        }

        for character_id in self.characters.clone() {
            let sprite = self.sprites.get_mut(&character_id).unwrap();
            if let Some(time) = sprite.scale_timer.as_mut() {
                *time -= 1.0/FPS;
                if *time > 0.0 {
                    continue
                }
                sprite.scale_timer.take();
                let delta = sprite.pending_scale;
                sprite.pending_scale = 0;
                if delta == 0 {
                    continue
                }
                sprite.scale = (sprite.scale as i32 + delta).max(1) as u32;
                sprite.loc.x -= SPRITE_WIDTH as f32 * delta as f32 * 0.5;
                sprite.loc.y -= SPRITE_WIDTH as f32 * delta as f32;
                if delta > 0 {
                    let cx = sprite.loc.x + SPRITE_WIDTH as f32 / 2.0;
                    let cy = sprite.loc.y + SPRITE_WIDTH as f32 / 2.0;
                    for dx in 0..SPRITE_WIDTH {
                        for dy in -1..SPRITE_WIDTH as i32-1 {
                            if true {
                                let x = sprite.loc.x as i32 + dx as i32 * sprite.scale as i32;
                                let y = sprite.loc.y as i32 + dy as i32 * sprite.scale as i32;
                                if self.collision_map.remove_rect(x, y, sprite.scale, sprite.scale).1 > 0 {
                                    let mut collider = [false; SPRITE_WIDTH*SPRITE_WIDTH];
                                    collider[0] = true;
                                    let mut new_sprite = Sprite::from_collider(collider, x as f32, y as f32, sprite.scale, Color::GREEN);
                                    let a = (cy-y as f32).atan2(cx - x as f32);
                                    new_sprite.velocity = Vector::new(a.cos() * 100.0/FPS, a.sin() * 100.0/FPS);
                                    new_sprites.push(new_sprite);
                                    for xx in 0..sprite.scale {
                                        for yy in 0..sprite.scale {
                                            let cx = sprite.loc.x as i32 + dx as i32 * sprite.scale as i32 + xx as i32;
                                            let cy = sprite.loc.y as i32 + dy as i32 * sprite.scale as i32 + yy as i32;
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
        for sprite in new_sprites {
            self.add_particle(sprite);
        }
    }

    fn draw(&mut self, gfx: &mut Graphics, x: i32, y: i32, width: u32, height: u32, scale: f32) {
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

        for xx in x/TILE_SIZE as i32 - 1..(x+width as i32)/TILE_SIZE as i32 + 1 {
            for yy in y/TILE_SIZE as i32 - 1..(y+height as i32) / TILE_SIZE as i32 + 1 {
                if !self.tile_cache.contains_key(&(xx, yy)) {
                    let mut tile = vec![0; (TILE_SIZE*TILE_SIZE*4) as usize];
                    for dx in 0..TILE_SIZE {
                        for dy in 0..TILE_SIZE {
                            if self.collision_map.check_point(xx * TILE_SIZE as i32+ dx as i32, yy * TILE_SIZE as i32 + dy as i32) {
                                let i = (dx + dy * TILE_SIZE) as usize * 4;
                                tile[i] = 0x00;
                                tile[i+1] = 0xff;
                                tile[i+2] = 0x00;
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
                    self.tile_cache.insert((xx, yy), tile);
                }
                let region = Rectangle::new(Vector::new((xx*TILE_SIZE as i32 - x) as f32, (yy*TILE_SIZE as i32 - y) as f32), Vector::new(TILE_SIZE as f32, TILE_SIZE as f32));
                gfx.draw_image(self.tile_cache.get(&(xx, yy)).as_ref().unwrap(), region);
            }
        }

    }
}

async fn app(window: Window, mut gfx: Graphics, mut input: Input) -> Result<()> {
    let sprites = image::load(std::io::Cursor::new(SPRITES), image::ImageFormat::Png).unwrap();
    let mut scene = Scene::new();
    let player_id = scene.add_character(Sprite::new(&sprites, 31, 2, 300.0, 600.0, 7, Color::BLUE));
    for x in 0..10 {
        scene.add_potion(Sprite::new(&sprites, 33, 13, (x as f32 * 2.0) *100.0+3.0, 688.0, 2, Color::RED), 1);
    }
    for x in 0..10 {
        scene.add_potion(Sprite::new(&sprites, 33, 13, (x as f32 * 2.0 + 1.0) *100.0, 688.0, 2, Color::BLUE), 1);
    }

    for x in 0..20 {
        scene.add_terrain(&Sprite::new(&sprites, 7, 5, (x*(SPRITE_WIDTH-2)*7) as f32, 800.0, 7, Color::BLUE));
        scene.add_terrain(&Sprite::new(&sprites, 7, 5, (x*(SPRITE_WIDTH-2)*7) as f32, 900.0, 7, Color::BLUE));
    }
    for x in 0..20 {
        scene.add_terrain(&Sprite::new(&sprites, 20, 15, (x*SPRITE_WIDTH*7) as f32, 499.0, 7, Color::BLUE));
    }

    let mut update_timer = Timer::time_per_second(FPS);
    let mut draw_timer = Timer::time_per_second(FPS);
    let mut camera = Vector::new(0.0, 0.0);
    let mut moving_left = false;
    let mut moving_right = false;
    loop {
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
        let vx = if input.key_down(Key::LShift) {
            92.0
        } else {
            46.0
        };
        if moving_right {
            let player = scene.sprites.get_mut(&player_id).unwrap();
            player.velocity.x = vx / FPS;
        } else if moving_left {
            let player = scene.sprites.get_mut(&player_id).unwrap();
            player.velocity.x = -vx / FPS;
        } else {
            let player = scene.sprites.get_mut(&player_id).unwrap();
            player.velocity.x = 0.0;
        }
        while update_timer.tick() {
            scene.step_physics();
        }
        if draw_timer.exhaust().is_some() {
            let player = scene.sprites.get_mut(&player_id).unwrap();
            camera.x = camera.x * 0.9 + (player.loc.x - 1920.0/2.0)*0.1;
            camera.y = camera.y * 0.9 + (player.loc.y - 1080.0/2.0) *0.1;
            gfx.clear(Color::WHITE);
            let scale = player.scale as f32 / 8.0;
            scene.draw(&mut gfx, camera.x as i32, camera.y as i32, 1920, 1080, scale);
            gfx.present(&window)?;
        }
    }
}
