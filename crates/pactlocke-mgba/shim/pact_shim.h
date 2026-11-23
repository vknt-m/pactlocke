/* Pactlocke's C shim over libmgba: one small, plain interface the Rust side
 * binds by hand. Always the GBA core, the HLE BIOS, an in-memory save and a
 * fake-epoch real-time clock.
 * SPDX-License-Identifier: GPL-3.0-or-later */
#ifndef PACT_SHIM_H
#define PACT_SHIM_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

typedef struct PactCore PactCore;

/* Make the C runtime's local time UTC for the whole process, so libmgba's
 * clock (which converts with localtime) reads the same on every machine.
 * Call once at start-up, before other threads exist. Idempotent. */
void pact_force_utc(void);

PactCore* pact_core_new(void);
void pact_core_free(PactCore* pc);

/* Copies the ROM into memory and powers on. False if it isn't a GBA ROM. */
bool pact_core_load_rom(PactCore* pc, const uint8_t* data, size_t len);
/* Fake-epoch clock: reads epoch_ms at power-on and advances with emulated
 * time, so a restored savestate reads its own time from it. */
void pact_core_set_clock(PactCore* pc, int64_t epoch_ms);
/* What the clock reads now, in Unix milliseconds. */
int64_t pact_core_clock_ms(PactCore* pc);

void pact_core_run_frame(PactCore* pc, uint32_t keys);

void pact_core_video_size(const PactCore* pc, uint32_t* width, uint32_t* height);
/* width*height pixels, 32 bits each, bytes R, G, B, X in memory. */
const uint32_t* pact_core_video_buffer(const PactCore* pc);

uint32_t pact_core_audio_sample_rate(const PactCore* pc);
/* Reads up to max_frames stereo frames (2*max_frames samples). */
size_t pact_core_audio_read(PactCore* pc, int16_t* out, size_t max_frames);

void pact_core_read(PactCore* pc, uint32_t addr, uint8_t* out, size_t len);
void pact_core_write(PactCore* pc, uint32_t addr, const uint8_t* data, size_t len);
void pact_core_write16(PactCore* pc, uint32_t addr, uint16_t value);
void pact_core_write32(PactCore* pc, uint32_t addr, uint32_t value);
void pact_core_patch_rom(PactCore* pc, uint32_t addr, const uint8_t* data, size_t len);
/* Display-only writes (OAM, OBJ tiles, OBJ palettes) applied as every frame
 * starts to draw, until replaced: records of a little-endian 32-bit address,
 * a 16-bit even length and the bytes. Copied. False if out of memory. */
bool pact_core_set_overlay(PactCore* pc, const uint8_t* data, size_t len);

size_t pact_core_state_size(PactCore* pc);
bool pact_core_save_state(PactCore* pc, uint8_t* out);
bool pact_core_load_state(PactCore* pc, const uint8_t* state);
/* Copies the save memory; *out must be released with pact_free. */
size_t pact_core_save_memory(PactCore* pc, uint8_t** out);
bool pact_core_restore_save_memory(PactCore* pc, const uint8_t* data, size_t len);
void pact_free(void* p);

#endif
