import { readdir, readFile } from 'node:fs/promises';
import { join } from 'node:path';

const objectsDir = '../pokered/data/maps/objects';

const hiddenSprites = [
    "PALLETTOWN_OAK",
    "VIRIDIANCITY_OLD_MAN_SLEEPY",
    "VIRIDIANCITY_OLD_MAN",
    "PEWTERCITY_SUPER_NERD1",
    "PEWTERCITY_YOUNGSTER",
    "CERULEANCITY_RIVAL",
    "CERULEANCITY_ROCKET",
    "CERULEANCITY_GUARD1",
    "CERULEANCITY_SUPER_NERD3",
    "CERULEANCITY_GUARD2",
    "SAFFRONCITY_ROCKET1",
    "SAFFRONCITY_ROCKET2",
    "SAFFRONCITY_ROCKET3",
    "SAFFRONCITY_ROCKET4",
    "SAFFRONCITY_ROCKET5",
    "SAFFRONCITY_ROCKET6",
    "SAFFRONCITY_ROCKET7",
    "SAFFRONCITY_SCIENTIST",
    "SAFFRONCITY_SILPH_WORKER_M",
    "SAFFRONCITY_SILPH_WORKER_F",
    "SAFFRONCITY_GENTLEMAN",
    "SAFFRONCITY_PIDGEOT",
    "SAFFRONCITY_ROCKER",
    "SAFFRONCITY_ROCKET8",
    "SAFFRONCITY_ROCKET9",
    "ROUTE2_MOON_STONE",
    "ROUTE2_HP_UP",
    "ROUTE4_TM_WHIRLWIND",
    "ROUTE9_TM_TELEPORT",
    "ROUTE12_SNORLAX",
    "ROUTE12_TM_PAY_DAY",
    "ROUTE12_IRON",
    "ROUTE15_TM_RAGE",
    "ROUTE16_SNORLAX",
    "ROUTE22_RIVAL1",
    "ROUTE22_RIVAL2",
    "ROUTE24_COOLTRAINER_M1",
    "ROUTE24_TM_THUNDER_WAVE",
    "ROUTE25_TM_SEISMIC_TOSS",
    "BLUESHOUSE_DAISY1",
    "BLUESHOUSE_DAISY2",
    "BLUESHOUSE_TOWN_MAP",
    "OAKSLAB_RIVAL",
    "OAKSLAB_CHARMANDER_POKE_BALL",
    "OAKSLAB_SQUIRTLE_POKE_BALL",
    "OAKSLAB_BULBASAUR_POKE_BALL",
    "OAKSLAB_OAK1",
    "OAKSLAB_POKEDEX1",
    "OAKSLAB_POKEDEX2",
    "OAKSLAB_OAK2",
    "VIRIDIANGYM_GIOVANNI",
    "VIRIDIANGYM_REVIVE",
    "MUSEUM1F_OLD_AMBER",
    "CERULEANCAVE1F_FULL_RESTORE",
    "CERULEANCAVE1F_MAX_ELIXER",
    "CERULEANCAVE1F_NUGGET",
    "POKEMONTOWER2F_RIVAL",
    "POKEMONTOWER3F_ESCAPE_ROPE",
    "POKEMONTOWER4F_ELIXER",
    "POKEMONTOWER4F_AWAKENING",
    "POKEMONTOWER4F_HP_UP",
    "POKEMONTOWER5F_NUGGET",
    "POKEMONTOWER6F_RARE_CANDY",
    "POKEMONTOWER6F_X_ACCURACY",
    "POKEMONTOWER7F_ROCKET1",
    "POKEMONTOWER7F_ROCKET2",
    "POKEMONTOWER7F_ROCKET3",
    "POKEMONTOWER7F_MR_FUJI",
    "MRFUJISHOUSE_MR_FUJI",
    "CELADONMANSION_ROOF_HOUSE_EEVEE_POKEBALL",
    "GAMECORNER_ROCKET",
    "WARDENSHOUSE_RARE_CANDY",
    "POKEMONMANSION1F_ESCAPE_ROPE",
    "POKEMONMANSION1F_CARBOS",
    "FIGHTINGDOJO_HITMONLEE_POKE_BALL",
    "FIGHTINGDOJO_HITMONCHAN_POKE_BALL",
    "SILPHCO1F_LINK_RECEPTIONIST",
    "POWERPLANT_VOLTORB1",
    "POWERPLANT_VOLTORB2",
    "POWERPLANT_VOLTORB3",
    "POWERPLANT_ELECTRODE1",
    "POWERPLANT_VOLTORB4",
    "POWERPLANT_VOLTORB5",
    "POWERPLANT_ELECTRODE2",
    "POWERPLANT_VOLTORB6",
    "POWERPLANT_ZAPDOS",
    "POWERPLANT_CARBOS",
    "POWERPLANT_HP_UP",
    "POWERPLANT_RARE_CANDY",
    "POWERPLANT_TM_THUNDER",
    "POWERPLANT_TM_REFLECT",
    "VICTORYROAD2F_MOLTRES",
    "VICTORYROAD2F_TM_SUBMISSION",
    "VICTORYROAD2F_FULL_HEAL",
    "VICTORYROAD2F_TM_MEGA_KICK",
    "VICTORYROAD2F_GUARD_SPEC",
    "VICTORYROAD2F_BOULDER3",
    "BILLSHOUSE_BILL_POKEMON",
    "BILLSHOUSE_BILL1",
    "BILLSHOUSE_BILL2",
    "VIRIDIANFOREST_ANTIDOTE",
    "VIRIDIANFOREST_POTION",
    "VIRIDIANFOREST_POKE_BALL",
    "MTMOON1F_POTION1",
    "MTMOON1F_MOON_STONE",
    "MTMOON1F_RARE_CANDY",
    "MTMOON1F_ESCAPE_ROPE",
    "MTMOON1F_POTION2",
    "MTMOON1F_TM_WATER_GUN",
    "MTMOONB2F_DOME_FOSSIL",
    "MTMOONB2F_HELIX_FOSSIL",
    "MTMOONB2F_HP_UP",
    "MTMOONB2F_TM_MEGA_PUNCH",
    "SSANNE2F_RIVAL",
    "SSANNE1FROOMS_TM_BODY_SLAM",
    "SSANNE2FROOMS_MAX_ETHER",
    "SSANNE2FROOMS_RARE_CANDY",
    "SSANNEB1FROOMS_ETHER",
    "SSANNEB1FROOMS_TM_REST",
    "SSANNEB1FROOMS_MAX_POTION",
    "VICTORYROAD3F_MAX_REVIVE",
    "VICTORYROAD3F_TM_EXPLOSION",
    "VICTORYROAD3F_BOULDER4",
    "ROCKETHIDEOUTB1F_ESCAPE_ROPE",
    "ROCKETHIDEOUTB1F_HYPER_POTION",
    "ROCKETHIDEOUTB2F_MOON_STONE",
    "ROCKETHIDEOUTB2F_NUGGET",
    "ROCKETHIDEOUTB2F_TM_HORN_DRILL",
    "ROCKETHIDEOUTB2F_SUPER_POTION",
    "ROCKETHIDEOUTB3F_TM_DOUBLE_EDGE",
    "ROCKETHIDEOUTB3F_RARE_CANDY",
    "ROCKETHIDEOUTB4F_GIOVANNI",
    "ROCKETHIDEOUTB4F_HP_UP",
    "ROCKETHIDEOUTB4F_TM_RAZOR_WIND",
    "ROCKETHIDEOUTB4F_IRON",
    "ROCKETHIDEOUTB4F_SILPH_SCOPE",
    "ROCKETHIDEOUTB4F_LIFT_KEY",
    "SILPHCO2F_SILPH_WORKER_F",
    "SILPHCO2F_SCIENTIST1",
    "SILPHCO2F_SCIENTIST2",
    "SILPHCO2F_ROCKET1",
    "SILPHCO2F_ROCKET2",
    "SILPHCO3F_ROCKET",
    "SILPHCO3F_SCIENTIST",
    "SILPHCO3F_HYPER_POTION",
    "SILPHCO4F_ROCKET1",
    "SILPHCO4F_SCIENTIST",
    "SILPHCO4F_ROCKET2",
    "SILPHCO4F_FULL_HEAL",
    "SILPHCO4F_MAX_REVIVE",
    "SILPHCO4F_ESCAPE_ROPE",
    "SILPHCO5F_ROCKET1",
    "SILPHCO5F_SCIENTIST",
    "SILPHCO5F_ROCKER",
    "SILPHCO5F_ROCKET2",
    "SILPHCO5F_TM_TAKE_DOWN",
    "SILPHCO5F_PROTEIN",
    "SILPHCO5F_CARD_KEY",
    "SILPHCO6F_ROCKET1",
    "SILPHCO6F_SCIENTIST",
    "SILPHCO6F_ROCKET2",
    "SILPHCO6F_HP_UP",
    "SILPHCO6F_X_ACCURACY",
    "SILPHCO7F_ROCKET1",
    "SILPHCO7F_SCIENTIST",
    "SILPHCO7F_ROCKET2",
    "SILPHCO7F_ROCKET3",
    "SILPHCO7F_RIVAL",
    "SILPHCO7F_CALCIUM",
    "SILPHCO7F_TM_SWORDS_DANCE",
    "SILPHCO7F_UNUSED",
    "SILPHCO8F_ROCKET1",
    "SILPHCO8F_SCIENTIST",
    "SILPHCO8F_ROCKET2",
    "SILPHCO9F_ROCKET1",
    "SILPHCO9F_SCIENTIST",
    "SILPHCO9F_ROCKET2",
    "SILPHCO10F_ROCKET",
    "SILPHCO10F_SCIENTIST",
    "SILPHCO10F_SILPH_WORKER_F",
    "SILPHCO10F_TM_EARTHQUAKE",
    "SILPHCO10F_RARE_CANDY",
    "SILPHCO10F_CARBOS",
    "SILPHCO11F_GIOVANNI",
    "SILPHCO11F_ROCKET1",
    "SILPHCO11F_ROCKET2",
    "UNUSEDMAPF4_UNUSED",
    "POKEMONMANSION2F_CALCIUM",
    "POKEMONMANSION3F_MAX_POTION",
    "POKEMONMANSION3F_IRON",
    "POKEMONMANSIONB1F_RARE_CANDY",
    "POKEMONMANSIONB1F_FULL_RESTORE",
    "POKEMONMANSIONB1F_TM_BLIZZARD",
    "POKEMONMANSIONB1F_TM_SOLARBEAM",
    "POKEMONMANSIONB1F_SECRET_KEY",
    "SAFARIZONEEAST_FULL_RESTORE",
    "SAFARIZONEEAST_MAX_RESTORE",
    "SAFARIZONEEAST_CARBOS",
    "SAFARIZONEEAST_TM_EGG_BOMB",
    "SAFARIZONENORTH_PROTEIN",
    "SAFARIZONENORTH_TM_SKULL_BASH",
    "SAFARIZONEWEST_MAX_POTION",
    "SAFARIZONEWEST_TM_DOUBLE_TEAM",
    "SAFARIZONEWEST_MAX_REVIVE",
    "SAFARIZONEWEST_GOLD_TEETH",
    "SAFARIZONECENTER_NUGGET",
    "CERULEANCAVE2F_PP_UP",
    "CERULEANCAVE2F_ULTRA_BALL",
    "CERULEANCAVE2F_FULL_RESTORE",
    "CERULEANCAVEB1F_MEWTWO",
    "CERULEANCAVEB1F_ULTRA_BALL",
    "CERULEANCAVEB1F_MAX_REVIVE",
    "VICTORYROAD1F_TM_SKY_ATTACK",
    "VICTORYROAD1F_RARE_CANDY",
    "CHAMPIONSROOM_OAK",
    "SEAFOAMISLANDS1F_BOULDER1",
    "SEAFOAMISLANDS1F_BOULDER2",
    "SEAFOAMISLANDSB1F_BOULDER1",
    "SEAFOAMISLANDSB1F_BOULDER2",
    "SEAFOAMISLANDSB2F_BOULDER1",
    "SEAFOAMISLANDSB2F_BOULDER2",
    "SEAFOAMISLANDSB3F_BOULDER2",
    "SEAFOAMISLANDSB3F_BOULDER3",
    "SEAFOAMISLANDSB3F_BOULDER5",
    "SEAFOAMISLANDSB3F_BOULDER6",
    "SEAFOAMISLANDSB4F_BOULDER1",
    "SEAFOAMISLANDSB4F_BOULDER2",
    "SEAFOAMISLANDSB4F_ARTICUNO"
]

async function extractSprites() {
    const files = await readdir(objectsDir);
    const asmFiles = files.filter(f => f.endsWith('.asm'));

    const allSprites = [];

    for (const file of asmFiles) {
        const mapName = file.replace('.asm', '');
        const content = await readFile(join(objectsDir, file), 'utf-8');
        const lines = content.split('\n');

        let id = 0;
        let inConstSection = false;

        for (const line of lines) {
            const withoutComments = line.split(';')[0];
            const trimmed = withoutComments.trim();

            if (trimmed === 'object_const_def') {
                inConstSection = true;
                id = 0;
                continue;
            }

            if (inConstSection && trimmed.startsWith('const_export ')) {
                id++;
                const name = trimmed.replace('const_export ', '').trim();
                const hiddenIndex = hiddenSprites.indexOf(name);

                const title = name
                    .toLowerCase()
                    .split('_')
                    .slice(1)
                    .flatMap(word => {
                        // Add space before trailing number
                        const match = word.match(/^([a-z]+)(\d+)$/);
                        if (match) {
                            return [match[1], match[2]];
                        }
                        return word;
                    })
                    .map(word => {
                        // Handle single letter gender markers
                        if (word === 'm') return 'Male';
                        if (word === 'f') return 'Female';

                        // Handle all-caps acronyms
                        if (word === 'tm' || word === 'hp' || word === 'pp') {
                            return word.toUpperCase();
                        }

                        // Default title case
                        return word.charAt(0).toUpperCase() + word.slice(1);
                    })
                    .join(' ');

                allSprites.push({ id, title, name, mapName, hiddenIndex });
            } else if (inConstSection && trimmed) {
                // content section ended
                break
            }
        }
    }

    return allSprites;
}



const sprites = await extractSprites();

// const groupedByMap = sprites.reduce((acc, sprite) => {
//     if (!acc[sprite.mapName]) {
//         acc[sprite.mapName] = [];
//     }
//     acc[sprite.mapName].push(sprite);
//     return acc;
// }, {});

// for (let [name, sprites] of Object.entries(groupedByMap)) {
//     const sprite_names = sprites.map(({ name }) => `MapSprite::${name}`).join(', ');
//     console.log(`Map::${name} => &[${sprite_names}],`)
// }

for (let { id, title, name, hiddenIndex } of sprites) {
    const sprite = hiddenIndex >= 0 ? `MapSprite::hidden(${id}, "${title}", ${hiddenIndex})` : `MapSprite::new(${id}, "${title}")`
    console.log(`pub const ${name}: MapSprite = ${sprite};`)
}

