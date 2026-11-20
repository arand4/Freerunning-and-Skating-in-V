//! What Faith's feet and hands touch, for her footstep and handstep sounds: GTA's own material
//! under her feet (and in front of her hands), from a short shape test ray, matched to Mirror's
//! Edge's sound sets (concrete, metal, air duct, wood, gantry, ladder, fence, cardboard, water,
//! glass). Mirror's Edge has no soft ground, so grass, dirt and sand sound like concrete.
use std::collections::HashSet;

use faith_move::greybox::Surface;
use glam::Vec3;

use crate::natives::*;
use crate::shv::ret_i32;
use crate::{log, native};

/// GTA V's materials: the hash a shape test gives, and the name (ScriptHookVDotNet3's
/// GTA.MaterialHash, which lists them all).
const MATERIALS: &[(u32, &str)] = &[
    (0x00000000, "None"),
    (0x004FE54A, "Blood"),
    (0x00CFEBB4, "Temp30"),
    (0x00F3B93B, "MetalHollowSmall"),
    (0x01F3C404, "Spine3"),
    (0x02D45692, "Temp11"),
    (0x03C5F90A, "Temp10"),
    (0x06BDCCA1, "HandLeft"),
    (0x07519E5D, "Cloth"),
    (0x0781FA34, "MetalChainLinkLarge"),
    (0x0789C7AB, "WoodSolidPolished"),
    (0x079E4953, "RockNoinst"),
    (0x0973AE44, "CarpetSolidDusty"),
    (0x0D4C07E2, "Marsh"),
    (0x0E18DFF5, "CardboardSheet"),
    (0x0E931A0E, "GlassBulletproof"),
    (0x10DD5498, "Tarmac"),
    (0x110F7216, "AnimalDefault"),
    (0x11436942, "Linoleum"),
    (0x129ECA2A, "MudPothole"),
    (0x13D5CB0D, "VfxMetalFlame"),
    (0x1567BF52, "ConcretePothole"),
    (0x19F81600, "Water"),
    (0x1BB7608F, "Temp23"),
    (0x1C42F3BC, "Paper"),
    (0x1CD01A28, "Buttocks"),
    (0x1E5E7A48, "SandDryDeep"),
    (0x1E6D775E, "SandCompact"),
    (0x1E94B2B7, "CarGlassOpaque"),
    (0x2114B37D, "WoodSolidMedium"),
    (0x216FF3F0, "ClaySoft"),
    (0x2257A573, "Cobblestone"),
    (0x22AD7B72, "Bushes"),
    (0x23500534, "SandstoneSolid"),
    (0x23EF48BC, "CarGlassMedium"),
    (0x241B6C19, "PhysPoolTableSurface"),
    (0x2473B1BF, "VfxMetalWaterTower"),
    (0x25612338, "PlasticHollow"),
    (0x26E885F4, "ShinLeft"),
    (0x27E49616, "CarpetSolid"),
    (0x2827CBD9, "SlattedBlinds"),
    (0x29E0C642, "Temp12"),
    (0x2B13503D, "WoodOldCreaky"),
    (0x2C848051, "Temp01"),
    (0x2CD49BD1, "MetalSolidLarge"),
    (0x2D6E26CD, "MetalChainLinkSmall"),
    (0x2D9C1E0D, "Stone"),
    (0x2E0ECF63, "PlasticHollowClear"),
    (0x30341454, "Foam"),
    (0x309F8BB7, "WoodSolidLarge"),
    (0x31B80AD6, "MetalCorrugatedIron"),
    (0x33C7D38F, "FreshMeat"),
    (0x3500F64A, "Temp27"),
    (0x363CBCD5, "SandWet"),
    (0x37E12A0B, "GlassShootThrough"),
    (0x38BBD00C, "GravelSmall"),
    (0x39FDE2BB, "PhysPoolTableCushion"),
    (0x3B7F59CE, "VfxWoodBeerBarrel"),
    (0x3B982E13, "Puddle"),
    (0x3C514932, "Temp17"),
    (0x3D285B19, "Temp09"),
    (0x3E4A6464, "LowerArmLeft"),
    (0x3F28ABAC, "EmissivePlastic"),
    (0x3F4163F1, "Temp06"),
    (0x3FD6150A, "CarGlassStrong"),
    (0x40475AB5, "Temp29"),
    (0x42251DC0, "MudDeep"),
    (0x4434DFE7, "ClayHard"),
    (0x461D0E9B, "WoodChipboard"),
    (0x46CA81E8, "Concrete"),
    (0x4A57FFCA, "CarGlassWeak"),
    (0x4CCC2AFF, "SandWetDeep"),
    (0x4F747B87, "Grass"),
    (0x4FFB413F, "FeatherPillow"),
    (0x5016ECD6, "Temp08"),
    (0x50384F9D, "PhysCarVoid"),
    (0x50B728DB, "Fiberglass"),
    (0x50C38DF2, "Temp18"),
    (0x553BE97C, "TvScreen"),
    (0x55E5AAEE, "BushesNoinst"),
    (0x56E0CA1D, "Spine2"),
    (0x596C55D1, "GlassOpaque"),
    (0x5978A2ED, "EmissiveGlass"),
    (0x5979C903, "UpperArmRight"),
    (0x5C67C62A, "SnowTarmac"),
    (0x5E73A22E, "MarshDeep"),
    (0x608ABC80, "SnowDeep"),
    (0x61826E7A, "MudSoft"),
    (0x61B1F936, "Brick"),
    (0x63545F03, "PhysNoFriction"),
    (0x666B1694, "Neck"),
    (0x689E0E75, "RoofTile"),
    (0x68FEB9FD, "MetalDuct"),
    (0x69F8EE36, "LowerArmRight"),
    (0x6E02C9AA, "Laminate"),
    (0x6E3DBFB8, "MetalHollowMedium"),
    (0x70726A55, "TarmacPothole"),
    (0x71AB3FEE, "PavingSlab"),
    (0x71E96559, "Temp03"),
    (0x7209440E, "SandstoneBrittle"),
    (0x72ADD5E0, "Temp04"),
    (0x72C668B6, "GravelTrainTrack"),
    (0x72D0C8E7, "FootLeft"),
    (0x73EF7697, "Marble"),
    (0x745D8E31, "Temp25"),
    (0x750D8481, "Temp24"),
    (0x76D9AC2F, "WoodHollowSmall"),
    (0x774441B4, "HandRight"),
    (0x77E08A22, "WoodLattice"),
    (0x78239B1A, "ConcretePavement"),
    (0x7830C8F1, "PhysCasterRusty"),
    (0x7D368D93, "MetalRailing"),
    (0x7EDC5571, "GravelLarge"),
    (0x7EFDF110, "CarSoftTopClear"),
    (0x7F630AE2, "CarPlastic"),
    (0x8070DCF9, "WoodHighFriction"),
    (0x8388FA6C, "StuntRampSurface"),
    (0x846BC4FF, "Plastic"),
    (0x85A387EB, "Temp20"),
    (0x85F61AC9, "PhysDynamicCoverBound"),
    (0x8653C6CD, "Leaves"),
    (0x87F87187, "PhysElectricMetal"),
    (0x8A1A9241, "Temp02"),
    (0x8C31B7EA, "MudHard"),
    (0x8C8308CA, "SnowLoose"),
    (0x8CE6E7D9, "IceTarmac"),
    (0x8D6C3ADC, "Spine0"),
    (0x8DBDD298, "CarEngine"),
    (0x8DD4EBB9, "TreeBark"),
    (0x8E4D8AFF, "SandTrack"),
    (0x8F9CD58F, "DirtTrack"),
    (0x9126E8CB, "PlasticClear"),
    (0x92B69883, "Hay"),
    (0x962C3F7B, "Default"),
    (0x96C43F1E, "Temp07"),
    (0x97476A9D, "Polystyrene"),
    (0x981E5200, "WoodHighDensity"),
    (0x995DA5E6, "CarGlassBulletproof"),
    (0x9B0A74CA, "PhysGolfBall"),
    (0x9E65F2A7, "Temp13"),
    (0x9E98536C, "Petrol"),
    (0x9F154729, "PlasticHighDensity"),
    (0x9F73E76C, "Perspex"),
    (0xA0EBF7E4, "SandLoose"),
    (0xA1961C15, "Temp15"),
    (0xA32DA7DA, "ClavicleRight"),
    (0xA402C0C0, "PhysBarbedWire"),
    (0xA5D57DD7, "Temp16"),
    (0xA8676EAF, "ClavicleLeft"),
    (0xA9BC4217, "MetalSolidSmall"),
    (0xA9DC9A13, "DriedMeat"),
    (0xAB87C845, "RoofFelt"),
    (0xAC038918, "CardboardBox"),
    (0xACC354B1, "CarpetFloorboard"),
    (0xACEE6610, "Temp05"),
    (0xAE64A1D4, "FootRight"),
    (0xB038852E, "PlasticHighDensityClear"),
    (0xB26EEFB0, "TarmacPainted"),
    (0xB34E900D, "GrassShort"),
    (0xB94A2EB5, "Ceramic"),
    (0xB9AF9A0E, "Temp28"),
    (0xBA428CAB, "PhysElectricFence"),
    (0xBB9CA6D8, "BrickPavement"),
    (0xBC0B421B, "Spine1"),
    (0xBC4922A4, "SandUnderwater"),
    (0xBD775456, "Temp26"),
    (0xBF59B491, "ConcreteDusty"),
    (0xC2251964, "Temp21"),
    (0xC59BC28A, "CarSoftTop"),
    (0xC72165D6, "BreezeBlock"),
    (0xC8D738E7, "WoodHollowLarge"),
    (0xC98F5B61, "Twigs"),
    (0xCBA23987, "SnowCompact"),
    (0xCDEB5023, "Rock"),
    (0xD0356F62, "Temp19"),
    (0xD125AA55, "Ice"),
    (0xD1461B30, "RubberHollow"),
    (0xD256ED46, "FiberglassHollow"),
    (0xD2FFA63D, "MetalManhole"),
    (0xD35443DE, "WoodFloorDusty"),
    (0xD36536C6, "PhysPoolTableBall"),
    (0xD42ACC0F, "Head"),
    (0xD48AA0F2, "MetalSolidRoadSurface"),
    (0xD63CCDDB, "Soil"),
    (0xD6CBF212, "VfxMetalSteam"),
    (0xD97F800A, "Temp14"),
    (0xD9B1CDE0, "Tarpaulin"),
    (0xDA2E9567, "Oil"),
    (0xDB059FFF, "Temp22"),
    (0xDD3CDCF9, "MetalHollowLarge"),
    (0xDDC7963F, "PlasterSolid"),
    (0xDDFF4E0C, "Leather"),
    (0xE194CB2A, "UpperArmLeft"),
    (0xE47A3E41, "GrassLong"),
    (0xE48CC7C1, "ThighLeft"),
    (0xE56A0745, "ShinRight"),
    (0xE699F485, "MetalGrille"),
    (0xE82A6F1C, "WoodSolidSmall"),
    (0xEA34E8F8, "MetalSolidMedium"),
    (0xEA3746BD, "WoodHollowMedium"),
    (0xEABD174E, "GravelDeep"),
    (0xED92FC47, "VfxMetalElectrified"),
    (0xED932E53, "Woodchips"),
    (0xEE9E1045, "PhysPedCapsule"),
    (0xEFB2DF09, "MudUnderwater"),
    (0xF0B2FF05, "PhysTennisBall"),
    (0xF0FC7AFE, "PlasterBrittle"),
    (0xF116BC2D, "RumbleStrip"),
    (0xF1DFF3F9, "ThighRight"),
    (0xF1F990E5, "PhysCaster"),
    (0xF2373DE9, "MetalGarageDoor"),
    (0xF7503F13, "Rubber"),
    (0xF8902AC8, "RockMossy"),
    (0xFA73FCA1, "CarMetal"),
];

/// The Mirror's Edge sound set for a GTA material.
pub fn surface_of(n: &str) -> Surface {
    if n == "MetalDuct" {
        Surface::Airduct
    } else if n == "MetalGrille" {
        Surface::MetalGantry
    } else if n.starts_with("MetalChainlink") || n == "MetalRailing" || n.contains("ElectricFence") || n.contains("BarbedWire") {
        Surface::MetalFence
    } else if n.starts_with("Metal") || n == "CarMetal" || n.starts_with("VfxMetal") || n == "CarEngine" || n == "PhysElectricMetal" {
        Surface::Metal
    } else if n.starts_with("Wood") || n == "VfxWoodBeerBarrel" || n == "TreeBark" {
        Surface::Wood
    } else if n.starts_with("Cardboard") || n == "Paper" {
        Surface::Cardboard
    } else if n.contains("Glass") || n == "Perspex" || n == "Tvscreen" {
        Surface::Glass
    } else if n == "Water" || n == "Puddle" || n.ends_with("Underwater") {
        Surface::Water
    } else {
        Surface::Concrete
    }
}

pub struct Probe {
    pub feet: Surface,
    pub hands: Surface,
    frame: u32,
    names: std::collections::HashMap<u32, &'static str>,
    seen: HashSet<u32>,
}

impl Probe {
    pub fn new() -> Probe {
        let names = MATERIALS.iter().copied().collect();
        Probe { feet: Surface::Concrete, hands: Surface::Concrete, frame: 0, names, seen: HashSet::new() }
    }

    /// Every few frames, one ray: down from just above her feet, or (alternately) forward from her
    /// chest. `feet` and `forward` in GTA coordinates. On a ladder both are the ladder.
    pub fn update(&mut self, ped: i32, feet: Vec3, forward: Vec3, on_ladder: bool) {
        if on_ladder {
            self.feet = Surface::MetalLadder;
            self.hands = Surface::MetalLadder;
            return;
        }
        self.frame = self.frame.wrapping_add(1);
        if self.frame % 4 != 0 {
            return;
        }
        let hands = self.frame % 8 == 0;
        let (a, b) = if hands {
            let c = feet + Vec3::Z * 1.3;
            (c, c + forward.normalize_or_zero() * 1.0)
        } else {
            (feet + Vec3::Z * 0.4, feet - Vec3::Z * 0.6)
        };
        let Some(hash) = material(ped, a, b) else { return };
        let name = self.names.get(&hash).copied();
        let s = name.map_or(Surface::Concrete, surface_of);
        if self.seen.insert(hash) {
            log!("faith: GTA material {} ({hash:#010x}) sounds like {s:?}", name.unwrap_or("unknown"));
        }
        if hands {
            self.hands = s;
        } else {
            self.feet = s;
        }
    }
}

/// The material hash a ray from `a` to `b` hits first (map, vehicles and objects; not `ped`).
fn material(ped: i32, a: Vec3, b: Vec3) -> Option<u32> {
    // 1 map, 2 vehicles, 16 objects; 7: the usual "options" value scripts pass.
    let h = ret_i32(native!(START_EXPENSIVE_SYNCHRONOUS_SHAPE_TEST_LOS_PROBE, a.x, a.y, a.z, b.x, b.y, b.z, 1 | 2 | 16, ped, 7));
    let mut hit = [0u64; 1];
    let mut end = [0u64; 3];
    let mut normal = [0u64; 3];
    let mut mat = [0u64; 1];
    let mut entity = [0u64; 1];
    let status = ret_i32(native!(
        GET_SHAPE_TEST_RESULT_INCLUDING_MATERIAL,
        h,
        hit.as_mut_ptr() as *const i8,
        end.as_mut_ptr() as *const i8,
        normal.as_mut_ptr() as *const i8,
        mat.as_mut_ptr() as *const i8,
        entity.as_mut_ptr() as *const i8
    ));
    (status == 2 && hit[0] as u32 != 0).then_some(mat[0] as u32)
}
