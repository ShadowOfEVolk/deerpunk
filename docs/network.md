# Сеть (протокол)

UDP multiplayer поверх **renet + renet_netcode**. Wire payload — те же байты SVC/CLC, что и in-process `Channel` (listen-server в одном процессе).

Исходники:

| Слой | Путь |
|---|---|
| Дискриминаторы SVC/CLC | `crates/common/network/protocol.rs` |
| Prefab RPC | `crates/common/network/prefab_rep.rs` |
| Field paths | `crates/common/network/field_path.rs` |
| UserCmd | `crates/common/network/usercmd.rs` |
| UserMessage | `crates/common/network/user_message.rs` |
| SoundEvent | `crates/common/network/sound_event.rs` |
| `NetworkEntity` | `crates/common/components/network_entity.rs` |
| Transport | `crates/host/net.rs` |
| In-process очереди | `crates/host/channel.rs` |
| Emit (server) | `crates/server/network/prefab_rep.rs` |
| Apply (client) | `crates/client/network/prefab_rep.rs` |
| MethodDB | `crates/host/engine_api/net.rs`, `crates/engine-api/src/net.rs` |

Все целые и float на wire — **little-endian**.

---

## Транспорт

| Параметр | Значение |
|---|---|
| `PROTOCOL_ID` | `0x4445455250554E4B` (ASCII `"DEERPUNK"` как u64) |
| Порт по умолчанию | `28015` (`DEFAULT_PORT`, cvar `sv_port`) |
| `max_clients` | 16 |
| Auth | OOB challenge + netcode **Secure** |
| Bind listen | `0.0.0.0:<port>` — **один** UDP, demux (`udp_demux.rs`) |

Роли `NetTransport`: `Idle` / `Host` / `Client`. Payload = `[u8 cmd][payload…]` — без дополнительного framing поверх renet message.

### Каналы renet (`DefaultChannel`)

**S2C** (`channel_for_svc`):

| SVC | Канал |
|---|---|
| `NetDelta` (44) | `Unreliable` |
| всё остальное | `ReliableOrdered` |

**C2S** (`send_c2s`):

| CLC | Канал |
|---|---|
| `UserCmd` (2) | `Unreliable` |
| остальное | `ReliableOrdered` |

Приём читает оба канала.

---

## Консоль

| Команда | Действие |
|---|---|
| `listen [port]` | UDP listen (`sv_port`, если порт не указан) |
| `connect <ip[:port]>` | remote client (порт по умолчанию 28015) |
| `net_disconnect` | сбросить renet transport |

При `map` / `set_server_enabled(true)` host **auto-listen**, если ещё не Host/Client.  
`connect` выключает локальный server (если был) и чистит client world.

---

## Connection lifecycle (auth → init → play)

Три фазы для **любого** клиента. Listen (slot 0) — тот же **init**/**play**, но S2C/C2S через in-process [`Channel`]; OOB **auth** только у UDP `connect`.

### Auth

1. Клиент: OOB `deerpunk_connect_ch` на `sv_port` → `ConnectToken` (`connect_auth.rs`).
2. Renet/netcode: `ClientAuthentication::Secure` / `ServerAuthentication::Secure`, `PROTOCOL_ID`.
3. До успешного auth **нет** игровых SVC/CLC.

### Init (один раз на слот)

**Host** после `ClientConnected`:

1. `server_player_connected(slot)` (серверный pawn).
2. `catchup = server_rep.tick` → **broadcast** существующим клиентам (**play**, не init).
3. **Только joiner** `slot`: `PrefabDef*` → `UserMessage` **Map** (клиент грузит BSP/worldspawn локально) → `SpawnPrefab*` (только `NetworkEntity`) → `UserMessage` LightStyle → SetView → **`SVC::InitDone`**.
4. После `InitDone` слот в **play** (`SlotPlayGate`); до этого `UserCmd` с UDP игнорируется.

**Remote client:**

1. Renet `connected` → фаза **init** (`in_game == false`, UserCmd не шлём).
2. Применяем init SVC; `PrefabDef` → кэш (spawn без диска).
3. `InitDone` → **play**, `in_game = true`.

### Play

- C2S: `UserCmd` (Unreliable).
- S2C: `NetDelta`, новые spawns, `UserMessage`, …
- Disconnect: `server_player_disconnected`, `players.disconnect_slot`, сброс gate.

### Slot ≠ entity

- **Connection slot** — индекс игрока на сервере (0 = local listen).
- **Entity id** — Flecs entity pawn’а; на каждого слота свой.
- **net_id** — `u16` репликации `NetworkEntity` (не slot и не entity id).

Gamelib: `api.net.set_player_entity(slot, ent)` → `v1:players:set_entity`.  
Движение: `api.net.get_usercmd(slot, …)` → `v1:players:get_usercmd`.  
Хранить один глобальный `player_entity` нельзя — только `slot → entity`.

### SetView

1. Host: entity слота → `server_rep.net_id_of` → `UserMessageRpc::set_view(net_id)`.
2. Local: `client_rep.set_view_net_id` напрямую (без UDP).
3. Remote: `send_s2c_to_slot` пакетом `UserMessage`.
4. Client: `view_net_id` → entity для камеры / extract.

---

## Общий конверт пакета

```text
u8  cmd          // SVC или CLC discriminant
…   payload      // зависит от cmd
```

Клиент: `Host::parse_server_message` (`match SVC`).  
`apply_local_s2c` = (опционально record) → parse. Channel / UDP / demo play кормят туда же.

Сервер: `Host::parse_client_message(slot, packet)` (`match CLC`). Channel + UDP c2s.

---

## SVC (server → client)

| Value | Name | Статус |
|---:|---|---|
| 0 | `Nop` | reserved |
| 1 | `Bad` | reserved (не эмитим) |
| 2 | `SoundPlay` | живой (сейчас в основном in-process `Channel`) |
| 3 | `SpawnPrefab` | живой, UDP |
| 4 | `NetDelta` | живой, UDP Unreliable |
| 5 | `NetDestroy` | живой, UDP |
| 6 | `UserMessage` | живой (`SetView`, `LightStyle`, …) |
| 7 | `PrefabDef` | **init** (authoritative prefab JSON) |
| 8 | `InitDone` | **init** → **play** (payload пустой) |
| 9 | `SoundStop` | живой |

### `InitDone` (8)

```text
(нет payload — только байт cmd)
```

### `PrefabDef` (7)

```text
u8   name_len
name_len × u8   utf-8 name (e.g. "player")
u32  json_len
json_len × u8   EntityDef JSON (same as `.prefab` file)
```

### Field list (Spawn / Delta)

```text
u8  count                         // max 255
count ×:
  u8   FieldPathId
  …    FieldValue                 // тип по path
```

| `FieldPathId` | u8 | Wire value |
|---|---:|---|
| `TransformPosition` | 1 | 3×f32 |
| `TransformRotation` | 2 | 4×f32 (quat) |
| `TransformScale` | 3 | 3×f32 |
| `CameraComponent` | 10 | empty (`PrefabDefault`) |
| `CharacterController` | 11 | empty |
| `SoundEmitter` | 12 | empty |
| `Material` | 13 | empty |
| `Worldspawn` | 14 | empty |
| `Mesh3D` | 15 | empty |
| `WorldspawnMap` | 16 | `u8 len` + UTF-8 |

Prefab JSON: `"hot": ["TransformComponent.position", …]`, `"cold": ["CameraComponent", …]` — парсинг в `FieldPathId::parse`.

### `SpawnPrefab` (43)

```text
u8   43
u16  net_id          // 0 = static/scene (worldspawn), иначе NetworkEntity
u8   prefab_len
[u8] prefab          // max 255
…    cold fields     // field list
…    hot fields      // field list
```

### `NetDelta` (44)

```text
u8   44
u16  net_id
…    fields          // field list (обычно hot)
```

### `NetDestroy` (45)

```text
u8   45
u16  net_id
```

### `UserMessage` (6)

```text
u8   6              // SVC::UserMessage
u8   msg_id         // как GoldSrc svc_usermessage; 1..255
[u8] data           // до конца пакета
```

Имена только при **`um_register` / `um_hook`** (server и client — **один порядок** регистрации).

Id **1+** — только через `um_register` (server **и** client, один порядок). Движок не резервирует имена.

DarkDemo (пример): `SetView` → `LightStyle` → `Map` = id 1, 2, 3. Payloads — `encode_set_view` / `encode_light_style` / `encode_map_name` в `user_message.rs`.

Пример SetView: `06 01 | net_id_lo net_id_hi`.

### `SoundPlay` (2)

Сервер выдаёт **`u32 sound_id`**; стоп — [`SoundStop`](#soundstop-9).

```text
u8    2
u32   sound_id
u8    flags          // SND_FLAG_2D | LOOP | NET_ENTITY
u16   net_id         // follow replicated entity (loop)
f32×3 origin
u8    channel
u8    volume         // 0..255 → linear
u8    name_len + event
u8    param_count + (name_len, name, f32 value)*
```

### `SoundStop` (9)

```text
u8    9
u32   sound_id
```

Сейчас в основном `channel` (listen); формат готов к S2C broadcast.

---

## CLC (client → server)

| Value | Name | Статус |
|---:|---|---|
| 0 | `Nop` | reserved |
| 1 | `Bad` | reserved (не эмитим) |
| 2 | `UserCmd` | живой |

### `UserCmd` (2)

```text
u8   2
[64] payload:
  u16 version = 1
  u16 flags
  f32 yaw
  f32 pitch
  u64 buttons
  f32 axes[4]          // 0=forward, 1=side, 2=up, 3=reserved
  u8  pad[24]          // zeros → 64 total
```

Кнопки:

| Бит | Имя |
|---|---|
| `1<<0` | `ATTACK` |
| `1<<1` | `JUMP` |
| `1<<2` | `DUCK` |
| `1<<3` | `FORWARD` |
| `1<<4` | `BACK` |
| `1<<5` | `MOVELEFT` |
| `1<<6` | `MOVERIGHT` |
| `1<<7` | `USE` |
| `1<<8` | `SPEED` |

---

## Prefab replication (hot / cold)

Компонент `NetworkEntity` (`#[repr(C)]`):

- `net_id: u16` — 0 = ещё не назначен; сервер выделяет с 1 (wrap, skip 0)
- `prefab[64]` + `prefab_len`
- `hot[16]` / `cold[16]` — `FieldPathId` как u8

Модель:

| | Cold | Hot |
|---|---|---|
| Когда | один раз в `SpawnPrefab` | snapshot при spawn + per-frame `NetDelta` при изменении |
| Типично | `PrefabDefault` (клиент берёт из локального prefab) или overrides (`WorldspawnMap`, transform) | position / rotation / scale |
| Канал | ReliableOrdered | Unreliable |

Правила:

- `net_id == 0` в SpawnPrefab = static (worldspawn); не кладётся в client `net_id → entity` map.
- Destroy: entity потерял `NetworkEntity` → `NetDestroyRpc`.
- Server tick order: worldspawns → new spawns → hot deltas → destroys.
- Baseline после map: `clear` + полный emit. Snapshot joiner’у — без clear.

ECS entity id **не** ездит по сети — только `net_id` + prefab name + fields.

---

## MethodDB / game API

| Name | Назначение |
|---|---|
| `v1:net:in_game` | 1 если server enabled **или** renet client connected |
| `v1:net:send_usercmd` | `(ptr, len≥64)` → CLC UserCmd (renet или Channel) |
| `v1:net:client_cmd` | `"map …"` / `"disconnect"` |
| `v1:players:set_entity` | `(slot, entity_u64)` — pawn слота |
| `v1:players:get_usercmd` | `(slot, out, len≥64)` |
| `v1:players:local_entity` | entity local pawn (0 если нет) |
| `v1:players:apply_local_view` | yaw/pitch/roll/eye/fov для локальной камеры |
| `v1:server:set_enabled` / `is_enabled` | listen-server on/off |
| `v1:move:snapshot` | axes + buttons для сборки UserCmd на клиенте |

Safe обёртка: `NetApi` в `crates/engine-api/src/net.rs`.

Gamelib на connect: spawn pawn → `set_player_entity(slot, ent)`.  
На frame: для каждого слота `get_usercmd(slot)` → движение.

---

## Поток данных (кратко)

```text
Client input
  → UserCmd (CLC/2, Unreliable)
  → Host players[slot]
  → game server_frame: get_usercmd(slot) → move pawn

Server world + NetworkEntity
  → SpawnPrefab / NetDelta / NetDestroy (SVC)
  → renet (ReliableOrdered / Unreliable)
  → client prefab_rep → local entities

set_player_entity(slot)
  → SetView UserMessage (net_id)
  → client camera follow
```

Listen-server: те же SVC/CLC через `Channel` для local client + UDP для remote.

---

## UserMessage API (gamelib)

Named SVC payloads beyond the fixed engine protocol (`SetView`, `LightStyle`, game-specific).

| Method | Назначение |
|---|---|
| `v1:net:msg_register(name, size)` | Server **и** client регистрируют имя; → msgid. `size` = max bytes или `-1` |
| `v1:net:msg_send(dest, name, data)` | `dest=0` broadcast; `dest=slot+1` одному игроку |
| `v1:net:msg_hook(name, cb)` | Client MsgFunc: `fn(name_ptr, name_len, data_ptr, data_len)` |

Safe: `NetApi::um_register` / `um_broadcast` / `um_send_slot` / `um_hook`.

Join init (gamelib): host вызывает `server_join_init(slot, phase)` — phase **0** после PrefabDef (Map), **1** после snapshot (LightStyle, SetView). См. `darkdemo-shared-net`.

Client hooks: `client_set_view_net_id`, `client_set_lightstyle`, `client_place_map` (`v1:client:*`).

---

## Connect auth (connectionless → Secure netcode)

Before renet handshake, client sends OOB on `sv_port`:

| Packet | Wire |
|---|---|
| Challenge | `ff ff ff ff` + `deerpunk_connect_ch` + `u64 client_id` + `u64 protocol_id` + sockaddr |
| Token | `ff ff ff ff` + `deerpunk_connect_tok` + `ConnectToken` (netcode wire) |

- Host: `ServerAuthentication::Secure`, key in `{gamedir}/netcode.key` (auto-created).
- Client: challenge → token → `ClientAuthentication::Secure`.
- Extra auth (password) not implemented yet — challenge always approved.
- Unknown client-facing IP: first challenge adds address to public list and host re-listens.
- Demux на game-сокете: не netcode challenge listener (отдельный bind убран).

## LAN discovery (connectionless)

GoldSrc-style OOB на **`sv_port`**. **Host:** один сокет — `FF FF FF FF` + тег → connect_auth / LAN info, иначе netcode. **Browser** в меню по-прежнему может `SO_REUSEADDR` на том же порту для приёма `lan_info`.

| Пакет | Wire |
|---|---|
| Query | `ff ff ff ff` + `deerpunk_lan_query` |
| Info | `ff ff ff ff` + `deerpunk_lan_info` + `\0` + `hostname\0` + `map\0` + `u16 LE port` + `u8 players` + `u8 max` |

- Host: `LanAdvertiser` шлёт info broadcast/localhost каждые ~1.5s (пока listen).
- Client menu: `lan_refresh` → bind reuse на `sv_port`, collect announces (TTL 8s).
- Lua: `eng_get_files_list("lan://servers")` → JSON `[{addr,ip,port,hostname,map,players,maxplayers},…]`.
- Команды: `lan_refresh`, `lan_stop`. Cvar `hostname` — имя в списке.

Исходники: `crates/host/lan.rs`, меню darkdemo `ui/widgets/main_menu.rml` (connect game).

---

## Debug

| Cvar | |
|---|---|
| `net_show 1` | Infobox: role, in/out B/s, rtt |
| `net_show 2` | + packet loss, player slots, remote addrs |

Inspector collection `players` — slot, local/remote, name, entity, netId, addr.

Latency/jitter simulation в renet нет — отдельная задача (delay queue).
