/* SPDX-License-Identifier: GPL-3.0-or-later */
#include "pact_shim.h"

#include <mgba/core/config.h>
#include <mgba/core/core.h>
#include <mgba/core/interface.h>
#include <mgba/core/log.h>
#include <mgba-util/audio-buffer.h>
#include <mgba-util/vfs.h>

#include <stdlib.h>
#include <string.h>
#include <time.h>

/* Room for several frames of audio, so 4x speed can't overflow between drains. */
#define PACT_AUDIO_BUFFER_FRAMES 8192
/* libmgba's GBA_AUDIO_VOLUME_MAX: the game's own mix, unscaled. */
#define PACT_MASTER_VOLUME 0x100

struct PactCore {
	struct mCore* core;
	mColor* video;
	unsigned width;
	unsigned height;
	bool loaded;
	/* Display-only writes applied as every frame starts to draw: records of
	 * a 32-bit address, a 16-bit length and the bytes. */
	uint8_t* overlay;
	size_t overlay_len;
	struct mCoreCallbacks callbacks;
};

/* At line 0, after the game's VBlank handler has copied its own OAM and
 * palettes over, so the overlay is what this frame draws. 16-bit stores:
 * OAM ignores 8-bit writes. */
static void pact_frame_started(void* context) {
	PactCore* pc = context;
	size_t at = 0;
	while (at + 6 <= pc->overlay_len) {
		const uint8_t* r = pc->overlay + at;
		uint32_t addr = (uint32_t) r[0] | (uint32_t) r[1] << 8 | (uint32_t) r[2] << 16 | (uint32_t) r[3] << 24;
		size_t len = (size_t) r[4] | (size_t) r[5] << 8;
		at += 6;
		if (at + len > pc->overlay_len) {
			break;
		}
		/* Display memory only: palette RAM, VRAM and OAM, never work RAM or IO. */
		if (addr < 0x05000000 || addr + len > 0x08000000) {
			at += len;
			continue;
		}
		for (size_t i = 0; i + 1 < len; i += 2) {
			uint16_t value = (uint16_t) (pc->overlay[at + i] | pc->overlay[at + i + 1] << 8);
			pc->core->busWrite16(pc->core, addr + (uint32_t) i, value);
		}
		at += len;
	}
}

static void pact_quiet_log(struct mLogger* logger, int category, enum mLogLevel level, const char* format, va_list args) {
	(void) logger;
	(void) category;
	(void) level;
	(void) format;
	(void) args;
}

static struct mLogger pact_quiet_logger = { .log = pact_quiet_log };

void pact_force_utc(void) {
#ifdef _WIN32
	_putenv_s("TZ", "UTC0");
	_tzset();
#else
	setenv("TZ", "UTC0", 1);
	tzset();
#endif
}

PactCore* pact_core_new(void) {
	mLogSetDefaultLogger(&pact_quiet_logger);
	PactCore* pc = calloc(1, sizeof(*pc));
	if (!pc) {
		return NULL;
	}
	pc->core = mCoreCreate(mPLATFORM_GBA);
	if (!pc->core) {
		free(pc);
		return NULL;
	}
	if (!pc->core->init(pc->core)) {
		free(pc->core);
		free(pc);
		return NULL;
	}
	mCoreInitConfig(pc->core, NULL);
	/* Always the HLE BIOS: never load a BIOS file. */
	mCoreConfigSetIntValue(&pc->core->config, "useBios", 0);
	/* Full volume. Loading a config with no "volume" option sets the core's
	 * master volume to 0, which silences every sample. */
	mCoreConfigSetIntValue(&pc->core->config, "volume", PACT_MASTER_VOLUME);
	mCoreLoadConfig(pc->core);

	pc->core->baseVideoSize(pc->core, &pc->width, &pc->height);
	pc->video = calloc((size_t) pc->width * pc->height, sizeof(mColor));
	if (!pc->video) {
		mCoreConfigDeinit(&pc->core->config);
		pc->core->deinit(pc->core);
		free(pc);
		return NULL;
	}
	pc->core->setVideoBuffer(pc->core, pc->video, pc->width);
	pc->callbacks.context = pc;
	pc->callbacks.videoFrameStarted = pact_frame_started;
	pc->core->addCoreCallbacks(pc->core, &pc->callbacks);
	pc->core->setAudioBufferSize(pc->core, PACT_AUDIO_BUFFER_FRAMES);
	return pc;
}

void pact_core_free(PactCore* pc) {
	if (!pc) {
		return;
	}
	mCoreConfigDeinit(&pc->core->config);
	pc->core->deinit(pc->core);
	free(pc->video);
	free(pc->overlay);
	free(pc);
}

bool pact_core_set_overlay(PactCore* pc, const uint8_t* data, size_t len) {
	uint8_t* copy = NULL;
	if (len) {
		copy = malloc(len);
		if (!copy) {
			return false;
		}
		memcpy(copy, data, len);
	}
	free(pc->overlay);
	pc->overlay = copy;
	pc->overlay_len = len;
	return true;
}

bool pact_core_load_rom(PactCore* pc, const uint8_t* data, size_t len) {
	struct VFile* vf = VFileMemChunk(data, len);
	if (!vf) {
		return false;
	}
	if (!pc->core->isROM(vf)) {
		vf->close(vf);
		return false;
	}
	if (pc->loaded) {
		pc->core->unloadROM(pc->core);
		pc->loaded = false;
	}
	if (!pc->core->loadROM(pc->core, vf)) {
		vf->close(vf);
		return false;
	}
	/* The save lives in memory only; nothing is written next to the ROM. */
	pc->core->loadSave(pc->core, VFileMemChunk(NULL, 0));
	pc->core->rtc.override = RTC_FAKE_EPOCH;
	pc->core->rtc.value = 0;
	pc->core->reset(pc->core);
	pc->loaded = true;
	return true;
}

void pact_core_set_clock(PactCore* pc, int64_t epoch_ms) {
	pc->core->rtc.override = RTC_FAKE_EPOCH;
	/* The fake epoch adds emulated time since power-on to the value, so take
	 * off what has already run to make the clock read epoch_ms now. */
	int64_t elapsed_ms = (int64_t) pc->core->frameCounter(pc->core) * pc->core->frameCycles(pc->core) * 1000LL /
	                     pc->core->frequency(pc->core);
	pc->core->rtc.value = epoch_ms - elapsed_ms;
}

void pact_core_run_frame(PactCore* pc, uint32_t keys) {
	pc->core->setKeys(pc->core, keys);
	pc->core->runFrame(pc->core);
}

void pact_core_video_size(const PactCore* pc, uint32_t* width, uint32_t* height) {
	*width = pc->width;
	*height = pc->height;
}

const uint32_t* pact_core_video_buffer(const PactCore* pc) {
	return (const uint32_t*) pc->video;
}

uint32_t pact_core_audio_sample_rate(const PactCore* pc) {
	return pc->core->audioSampleRate(pc->core);
}

size_t pact_core_audio_read(PactCore* pc, int16_t* out, size_t max_frames) {
	struct mAudioBuffer* buffer = pc->core->getAudioBuffer(pc->core);
	return mAudioBufferRead(buffer, out, max_frames);
}

void pact_core_read(PactCore* pc, uint32_t addr, uint8_t* out, size_t len) {
	for (size_t i = 0; i < len; ++i) {
		out[i] = (uint8_t) pc->core->busRead8(pc->core, addr + (uint32_t) i);
	}
}

void pact_core_write(PactCore* pc, uint32_t addr, const uint8_t* data, size_t len) {
	for (size_t i = 0; i < len; ++i) {
		pc->core->busWrite8(pc->core, addr + (uint32_t) i, data[i]);
	}
}

void pact_core_write16(PactCore* pc, uint32_t addr, uint16_t value) {
	pc->core->busWrite16(pc->core, addr, value);
}

void pact_core_write32(PactCore* pc, uint32_t addr, uint32_t value) {
	pc->core->busWrite32(pc->core, addr, value);
}

void pact_core_patch_rom(PactCore* pc, uint32_t addr, const uint8_t* data, size_t len) {
	for (size_t i = 0; i < len; ++i) {
		pc->core->rawWrite8(pc->core, addr + (uint32_t) i, -1, data[i]);
	}
}

size_t pact_core_state_size(PactCore* pc) {
	return pc->core->stateSize(pc->core);
}

bool pact_core_save_state(PactCore* pc, uint8_t* out) {
	return pc->core->saveState(pc->core, out);
}

bool pact_core_load_state(PactCore* pc, const uint8_t* state) {
	return pc->core->loadState(pc->core, state);
}

size_t pact_core_save_memory(PactCore* pc, uint8_t** out) {
	void* sram = NULL;
	size_t size = pc->core->savedataClone(pc->core, &sram);
	*out = sram;
	return size;
}

bool pact_core_restore_save_memory(PactCore* pc, const uint8_t* data, size_t len) {
	/* Write the data back into the save itself, not a temporary overlay. */
	return pc->core->savedataRestore(pc->core, data, len, true);
}

void pact_free(void* p) {
	free(p);
}
