// Аппаратный видеокодер компьютера для потока кадров synlink: libavcodec
// (h264/hevc_vaapi — Intel/AMD, h264/hevc_nvenc — NVIDIA).
//
// VAAPI: входные кадры — VA-поверхности NV12 пула кодера, экспортированные как
// dma-buf (av_hwframe_map → DRM PRIME); GPU композитора рисует в них плоскости
// Y и UV напрямую, копий через CPU нет. NVENC: кадр NV12 приходит из памяти
// (композитор рисует на другом GPU), кодер сам загружает его в видеокарту.
//
// libavcodec/libavutil загружаются через dlopen: synwm не зависит от ffmpeg,
// без него просто нет видео на компьютере. Один кадр в пути: `fenc_encode`
// отдаёт кадр и забирает готовый пакет (без B-кадров, async_depth=1).
#include <dlfcn.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include <libavcodec/avcodec.h>
#include <libavutil/hwcontext.h>
#include <libavutil/hwcontext_drm.h>
#include <libavutil/opt.h>

#define STR2(x) #x
#define STR(x) STR2(x)

// Функции libav*, которые нужны кодеру.
#define AV_FUNCS(X)                                                                                  \
    X(avutil, av_hwdevice_ctx_create, int, (AVBufferRef **, enum AVHWDeviceType, const char *,      \
                                              AVDictionary *, int))                                  \
    X(avutil, av_hwframe_ctx_alloc, AVBufferRef *, (AVBufferRef *))                                  \
    X(avutil, av_hwframe_ctx_init, int, (AVBufferRef *))                                             \
    X(avutil, av_hwframe_get_buffer, int, (AVBufferRef *, AVFrame *, int))                           \
    X(avutil, av_hwframe_map, int, (AVFrame *, const AVFrame *, int))                                \
    X(avutil, av_buffer_ref, AVBufferRef *, (const AVBufferRef *))                                   \
    X(avutil, av_buffer_unref, void, (AVBufferRef **))                                               \
    X(avutil, av_frame_alloc, AVFrame *, (void))                                                     \
    X(avutil, av_frame_free, void, (AVFrame **))                                                     \
    X(avutil, av_frame_get_buffer, int, (AVFrame *, int))                                            \
    X(avutil, av_dict_set, int, (AVDictionary **, const char *, const char *, int))                  \
    X(avutil, av_dict_free, void, (AVDictionary **))                                                 \
    X(avutil, av_strerror, int, (int, char *, size_t))                                               \
    X(avutil, av_opt_set_int, int, (void *, const char *, int64_t, int))                             \
    X(avcodec, avcodec_find_encoder_by_name, const AVCodec *, (const char *))                        \
    X(avcodec, avcodec_alloc_context3, AVCodecContext *, (const AVCodec *))                          \
    X(avcodec, avcodec_open2, int, (AVCodecContext *, const AVCodec *, AVDictionary **))             \
    X(avcodec, avcodec_free_context, void, (AVCodecContext **))                                      \
    X(avcodec, avcodec_send_frame, int, (AVCodecContext *, const AVFrame *))                         \
    X(avcodec, avcodec_receive_packet, int, (AVCodecContext *, AVPacket *))                          \
    X(avcodec, av_packet_alloc, AVPacket *, (void))                                                  \
    X(avcodec, av_packet_free, void, (AVPacket **))                                                  \
    X(avcodec, av_packet_unref, void, (AVPacket *))

#define DECL(lib, name, ret, args) static ret(*p_##name) args;
AV_FUNCS(DECL)

static int libs_loaded = -1;

static int load_libs(char *err, int errlen) {
    if (libs_loaded >= 0) {
        if (!libs_loaded) snprintf(err, errlen, "нет libavcodec " STR(LIBAVCODEC_VERSION_MAJOR));
        return libs_loaded;
    }
    void *avutil = dlopen("libavutil.so." STR(LIBAVUTIL_VERSION_MAJOR), RTLD_NOW | RTLD_LOCAL);
    void *avcodec = dlopen("libavcodec.so." STR(LIBAVCODEC_VERSION_MAJOR), RTLD_NOW | RTLD_LOCAL);
    libs_loaded = 0;
    if (!avutil || !avcodec) {
        snprintf(err, errlen, "нет libavcodec." STR(LIBAVCODEC_VERSION_MAJOR) "/libavutil." STR(LIBAVUTIL_VERSION_MAJOR) ": %s", dlerror());
        return 0;
    }
#define LOAD(lib, name, ret, args)                                                                   \
    if (!(p_##name = (ret(*) args)dlsym(lib, #name))) {                                              \
        snprintf(err, errlen, "нет %s в lib" #lib, #name);                                           \
        return 0;                                                                                    \
    }
    AV_FUNCS(LOAD)
    libs_loaded = 1;
    return 1;
}

// Плоскости входного буфера: Y и UV (dma-buf).
struct fenc_planes {
    int fd[2];
    uint32_t offset[2];
    uint32_t pitch[2];
    uint64_t modifier;
};

#define MAX_IN 4

struct fenc {
    int vaapi;
    AVCodecContext *ctx;
    AVBufferRef *device, *frames;
    AVFrame *in[MAX_IN];   // VA-поверхности
    AVFrame *drm[MAX_IN];  // их отображение в dma-buf (держит fd)
    uint32_t n_in, next;
    AVPacket *pkt;
    uint8_t *out;
    size_t out_cap;
    uint32_t w, h;
};

static void averr(char *err, int errlen, const char *what, int rc) {
    char s[128] = "";
    p_av_strerror(rc, s, sizeof s);
    snprintf(err, errlen, "%s: %s", what, s);
}

void fenc_close(struct fenc *e) {
    if (!e) return;
    if (e->ctx) p_avcodec_free_context(&e->ctx);
    for (uint32_t i = 0; i < e->n_in; i++) {
        if (e->drm[i]) p_av_frame_free(&e->drm[i]);
        if (e->in[i]) p_av_frame_free(&e->in[i]);
    }
    if (e->frames) p_av_buffer_unref(&e->frames);
    if (e->device) p_av_buffer_unref(&e->device);
    if (e->pkt) p_av_packet_free(&e->pkt);
    free(e->out);
    free(e);
}

// kind: 0 — VAAPI на узле `dev` (render node GPU композитора), 1 — NVENC.
struct fenc *fenc_open(int kind, const char *dev, uint32_t w, uint32_t h, int hevc, uint32_t bitrate, uint32_t fps,
                       uint32_t *n_inputs, char *err, int errlen) {
    if (!load_libs(err, errlen)) return NULL;
    struct fenc *e = calloc(1, sizeof *e);
    e->vaapi = kind == 0;
    e->w = w;
    e->h = h;
    const char *name = e->vaapi ? (hevc ? "hevc_vaapi" : "h264_vaapi") : (hevc ? "hevc_nvenc" : "h264_nvenc");
    const AVCodec *codec = p_avcodec_find_encoder_by_name(name);
    if (!codec) {
        snprintf(err, errlen, "в libavcodec нет кодера %s", name);
        goto fail;
    }
    int rc;
    if (e->vaapi) {
        if ((rc = p_av_hwdevice_ctx_create(&e->device, AV_HWDEVICE_TYPE_VAAPI, dev, NULL, 0)) < 0) {
            averr(err, errlen, "VAAPI", rc);
            goto fail;
        }
        e->frames = p_av_hwframe_ctx_alloc(e->device);
        AVHWFramesContext *fc = (AVHWFramesContext *)e->frames->data;
        fc->format = AV_PIX_FMT_VAAPI;
        fc->sw_format = AV_PIX_FMT_NV12;
        fc->width = w;
        fc->height = h;
        fc->initial_pool_size = MAX_IN + 2;
        if ((rc = p_av_hwframe_ctx_init(e->frames)) < 0) {
            averr(err, errlen, "пул VA-поверхностей", rc);
            goto fail;
        }
    }
    e->ctx = p_avcodec_alloc_context3(codec);
    AVCodecContext *c = e->ctx;
    c->width = w;
    c->height = h;
    c->time_base = (AVRational){1, 1000000};
    c->framerate = (AVRational){fps, 1};
    c->bit_rate = bitrate;
    c->rc_max_rate = bitrate;
    // Буфер на ~0,25 с: всплеск после покоя не душится, задержка мала.
    c->rc_buffer_size = bitrate / 4;
    c->gop_size = 600;  // ключевые — по запросу зрителя и раз в 10 с
    c->max_b_frames = 0;
    c->color_range = AVCOL_RANGE_JPEG;  // как пишут шейдеры: BT.709, полный диапазон
    c->colorspace = AVCOL_SPC_BT709;
    c->color_primaries = AVCOL_PRI_BT709;
    c->color_trc = AVCOL_TRC_BT709;
    AVDictionary *opts = NULL;
    if (e->vaapi) {
        c->pix_fmt = AV_PIX_FMT_VAAPI;
        c->hw_frames_ctx = p_av_buffer_ref(e->frames);
        p_av_dict_set(&opts, "async_depth", "1", 0);
        p_av_dict_set(&opts, "rc_mode", "VBR", 0);
    } else {
        c->pix_fmt = AV_PIX_FMT_NV12;
        p_av_dict_set(&opts, "preset", "p2", 0);
        p_av_dict_set(&opts, "tune", "ull", 0);
        p_av_dict_set(&opts, "zerolatency", "1", 0);
        p_av_dict_set(&opts, "delay", "0", 0);
        p_av_dict_set(&opts, "rc", "vbr", 0);
        p_av_dict_set(&opts, "forced-idr", "1", 0);
    }
    rc = p_avcodec_open2(c, codec, &opts);
    p_av_dict_free(&opts);
    if (rc < 0) {
        averr(err, errlen, name, rc);
        goto fail;
    }
    e->pkt = p_av_packet_alloc();
    if (e->vaapi) {
        e->n_in = MAX_IN;
        for (uint32_t i = 0; i < e->n_in; i++) {
            e->in[i] = p_av_frame_alloc();
            if ((rc = p_av_hwframe_get_buffer(e->frames, e->in[i], 0)) < 0) {
                averr(err, errlen, "VA-поверхность", rc);
                goto fail;
            }
            e->drm[i] = p_av_frame_alloc();
            e->drm[i]->format = AV_PIX_FMT_DRM_PRIME;
            if ((rc = p_av_hwframe_map(e->drm[i], e->in[i], AV_HWFRAME_MAP_READ | AV_HWFRAME_MAP_WRITE)) < 0) {
                averr(err, errlen, "экспорт VA-поверхности в dma-buf", rc);
                goto fail;
            }
        }
    } else {
        e->n_in = 1;
    }
    *n_inputs = e->n_in;
    return e;
fail:
    fenc_close(e);
    return NULL;
}

// Плоскости входа `i` (VAAPI): 0 — успех.
int fenc_input(struct fenc *e, uint32_t i, struct fenc_planes *p) {
    if (!e->vaapi || i >= e->n_in) return -1;
    const AVDRMFrameDescriptor *d = (const AVDRMFrameDescriptor *)e->drm[i]->data[0];
    const AVDRMPlaneDescriptor *pl[2];
    if (d->nb_layers >= 2) {
        pl[0] = &d->layers[0].planes[0];
        pl[1] = &d->layers[1].planes[0];
    } else if (d->nb_layers == 1 && d->layers[0].nb_planes >= 2) {
        pl[0] = &d->layers[0].planes[0];
        pl[1] = &d->layers[0].planes[1];
    } else {
        return -1;
    }
    for (int k = 0; k < 2; k++) {
        p->fd[k] = d->objects[pl[k]->object_index].fd;
        p->offset[k] = (uint32_t)pl[k]->offset;
        p->pitch[k] = (uint32_t)pl[k]->pitch;
    }
    p->modifier = d->objects[pl[0]->object_index].format_modifier;
    return 0;
}

// Следующий вход по кругу (кадр в пути один — предыдущий уже закодирован).
int fenc_next_input(struct fenc *e) {
    uint32_t i = e->next;
    e->next = (e->next + 1) % e->n_in;
    return (int)i;
}

void fenc_set_bitrate(struct fenc *e, uint32_t bitrate) {
    // Битрейт на ходу меняет не каждый кодер — пусть пробует.
    e->ctx->bit_rate = bitrate;
    e->ctx->rc_max_rate = bitrate;
}

// Закодировать вход `i` (VAAPI) или кадр NV12 из памяти (`y`, `uv`, общий шаг
// строки `stride`; NVENC). Возвращает длину пакета (`*out`) или -код ошибки.
long fenc_encode(struct fenc *e, uint32_t i, const uint8_t *y, const uint8_t *uv, uint32_t stride, int force_key,
                 uint64_t ts_us, const uint8_t **out, int *key) {
    AVFrame *f;
    AVFrame *own = NULL;
    if (e->vaapi) {
        f = e->in[i];
    } else {
        own = p_av_frame_alloc();
        own->format = AV_PIX_FMT_NV12;
        own->width = e->w;
        own->height = e->h;
        if (p_av_frame_get_buffer(own, 0) < 0) {
            p_av_frame_free(&own);
            return -12;
        }
        for (uint32_t r = 0; r < e->h; r++) memcpy(own->data[0] + r * own->linesize[0], y + r * stride, e->w);
        for (uint32_t r = 0; r < e->h / 2; r++) memcpy(own->data[1] + r * own->linesize[1], uv + r * stride, e->w);
        f = own;
    }
    f->pts = (int64_t)ts_us;
    f->pict_type = force_key ? AV_PICTURE_TYPE_I : AV_PICTURE_TYPE_NONE;
    if (force_key)
        f->flags |= AV_FRAME_FLAG_KEY;
    else
        f->flags &= ~AV_FRAME_FLAG_KEY;
    int rc = p_avcodec_send_frame(e->ctx, f);
    if (own) p_av_frame_free(&own);
    if (rc < 0) return rc;
    size_t len = 0;
    *key = 0;
    // Пакеты этого кадра (обычно один).
    for (;;) {
        rc = p_avcodec_receive_packet(e->ctx, e->pkt);
        if (rc < 0) break;
        if (len + e->pkt->size > e->out_cap) {
            e->out_cap = (len + e->pkt->size) * 2;
            e->out = realloc(e->out, e->out_cap);
        }
        memcpy(e->out + len, e->pkt->data, e->pkt->size);
        len += e->pkt->size;
        if (e->pkt->flags & AV_PKT_FLAG_KEY) *key = 1;
        p_av_packet_unref(e->pkt);
    }
    if (rc != AVERROR(EAGAIN) && rc != AVERROR_EOF) return rc;
    *out = e->out;
    return (long)len;
}
