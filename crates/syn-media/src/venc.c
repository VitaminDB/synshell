// Аппаратный видеокодер V4L2 M2M (stateful, mplane) для записи видео «Камеры» (по образцу synwm/src/v4l2enc.c).
//
// Вход — NV12 в dma-buf из dma-heap (Android-драйверы вроде Qualcomm msm_vidc
// не дают на входе буферы MMAP), кадр в них копирует CPU (venc_in_fd + mmap);
// выход — H.264/HEVC. Один кадр в пути: `venc_encode` отдаёт вход и ждёт готовый пакет.
// Цвет — как у кадров камеры (JFIF): BT.601, полный диапазон.
#include <errno.h>
#include <fcntl.h>
#include <linux/dma-heap.h>
#include <linux/videodev2.h>
#include <poll.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <sys/mman.h>
#include <unistd.h>

#define MAXBUF 32

struct venc_info {
    uint32_t width, height;   // размер кадра
    uint32_t stride;          // байт в строке Y и UV
    uint32_t y_scanlines;     // строк Y-плоскости (UV начинается с stride * y_scanlines)
    uint32_t in_size;         // размер входного буфера
    uint32_t in_count;        // входных буферов
};

struct venc {
    int fd;
    struct venc_info info;
    int in_fd[MAXBUF];
    int in_busy[MAXBUF];
    int cap_fd[MAXBUF];
    uint8_t *cap_map[MAXBUF];
    uint32_t cap_size;
    uint32_t cap_count;
    uint8_t *out;
    size_t out_cap;
};

static void seterr(char *err, int len, const char *what) {
    if (err && len > 0) snprintf(err, len, "%s: %s", what, strerror(errno));
}

static int heap_alloc(size_t size) {
    const char *heaps[] = {"/dev/dma_heap/system", "/dev/dma_heap/qcom,system"};
    for (unsigned i = 0; i < 2; i++) {
        int h = open(heaps[i], O_RDWR | O_CLOEXEC);
        if (h < 0) continue;
        struct dma_heap_allocation_data a = {.len = size, .fd_flags = O_RDWR | O_CLOEXEC};
        int r = ioctl(h, DMA_HEAP_IOCTL_ALLOC, &a);
        close(h);
        if (r == 0) return a.fd;
    }
    return -1;
}

static void ctrl(int fd, uint32_t id, int32_t v) {
    struct v4l2_control c = {.id = id, .value = v};
    ioctl(fd, VIDIOC_S_CTRL, &c);
}

void venc_close(struct venc *e) {
    if (!e) return;
    if (e->fd >= 0) {
        int t = V4L2_BUF_TYPE_VIDEO_OUTPUT_MPLANE;
        ioctl(e->fd, VIDIOC_STREAMOFF, &t);
        t = V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE;
        ioctl(e->fd, VIDIOC_STREAMOFF, &t);
    }
    for (uint32_t i = 0; i < MAXBUF; i++) {
        if (e->in_fd[i] >= 0) close(e->in_fd[i]);
        if (e->cap_map[i] && e->cap_map[i] != MAP_FAILED) munmap(e->cap_map[i], e->cap_size);
        if (e->cap_fd[i] >= 0) close(e->cap_fd[i]);
    }
    if (e->fd >= 0) close(e->fd);
    free(e->out);
    free(e);
}

static int queue_cap(struct venc *e, uint32_t i) {
    struct v4l2_plane pl = {.m.fd = e->cap_fd[i], .length = e->cap_size};
    struct v4l2_buffer b = {.index = i, .type = V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE, .memory = V4L2_MEMORY_DMABUF,
                            .length = 1, .m.planes = &pl};
    return ioctl(e->fd, VIDIOC_QBUF, &b);
}

// Найти кодер (устройство M2M с выходом H.264/HEVC): `dev` = NULL — перебрать /dev/video*.
static int find_encoder(const char *dev, uint32_t codec) {
    char path[32];
    for (int n = dev ? -1 : 0; n < 64; n++) {
        const char *p = dev;
        if (!dev) {
            snprintf(path, sizeof path, "/dev/video%d", n);
            p = path;
        }
        int fd = open(p, O_RDWR | O_NONBLOCK | O_CLOEXEC);
        if (fd >= 0) {
            struct v4l2_capability cap = {0};
            // M2M: либо флаг M2M_MPLANE, либо (как у msm_vidc) вход и выход mplane вместе.
            const uint32_t both = V4L2_CAP_VIDEO_CAPTURE_MPLANE | V4L2_CAP_VIDEO_OUTPUT_MPLANE;
            if (ioctl(fd, VIDIOC_QUERYCAP, &cap) == 0 &&
                ((cap.device_caps & V4L2_CAP_VIDEO_M2M_MPLANE) || (cap.capabilities & V4L2_CAP_VIDEO_M2M_MPLANE) ||
                 ((((cap.capabilities & V4L2_CAP_DEVICE_CAPS) ? cap.device_caps : cap.capabilities) & both) == both))) {
                for (uint32_t i = 0;; i++) {
                    struct v4l2_fmtdesc d = {.index = i, .type = V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE};
                    if (ioctl(fd, VIDIOC_ENUM_FMT, &d) < 0) break;
                    if (d.pixelformat == codec) return fd;
                }
            }
            close(fd);
        }
        if (dev) break;
    }
    return -1;
}

struct venc *venc_open(const char *dev, uint32_t w, uint32_t h, int hevc, uint32_t bitrate, uint32_t fps,
                       uint32_t gop, struct venc_info *info, char *err, int errlen) {
    struct venc *e = calloc(1, sizeof *e);
    e->fd = -1;
    for (int i = 0; i < MAXBUF; i++) e->in_fd[i] = e->cap_fd[i] = -1;
    uint32_t codec = hevc ? V4L2_PIX_FMT_HEVC : V4L2_PIX_FMT_H264;
    e->fd = find_encoder(dev, codec);
    if (e->fd < 0) {
        errno = ENODEV;
        seterr(err, errlen, hevc ? "нет кодера HEVC" : "нет кодера H.264");
        venc_close(e);
        return NULL;
    }
    int fd = e->fd;
    struct v4l2_format cf = {.type = V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE};
    cf.fmt.pix_mp.pixelformat = codec;
    cf.fmt.pix_mp.width = w;
    cf.fmt.pix_mp.height = h;
    cf.fmt.pix_mp.num_planes = 1;
    cf.fmt.pix_mp.plane_fmt[0].sizeimage = w * h * 3 / 2;
    if (ioctl(fd, VIDIOC_S_FMT, &cf) < 0) { seterr(err, errlen, "S_FMT выход"); venc_close(e); return NULL; }
    struct v4l2_format of = {.type = V4L2_BUF_TYPE_VIDEO_OUTPUT_MPLANE};
    of.fmt.pix_mp.pixelformat = V4L2_PIX_FMT_NV12;
    of.fmt.pix_mp.width = w;
    of.fmt.pix_mp.height = h;
    of.fmt.pix_mp.num_planes = 1;
    // Кадры камеры — JFIF: BT.601, полный диапазон (кодер пишет это в VUI потока).
    of.fmt.pix_mp.colorspace = V4L2_COLORSPACE_JPEG;
    of.fmt.pix_mp.ycbcr_enc = V4L2_YCBCR_ENC_601;
    of.fmt.pix_mp.quantization = V4L2_QUANTIZATION_FULL_RANGE;
    of.fmt.pix_mp.xfer_func = V4L2_XFER_FUNC_SRGB;
    if (ioctl(fd, VIDIOC_S_FMT, &of) < 0) { seterr(err, errlen, "S_FMT вход"); venc_close(e); return NULL; }
    if (ioctl(fd, VIDIOC_G_FMT, &cf) < 0) { seterr(err, errlen, "G_FMT выход"); venc_close(e); return NULL; }
    e->info.width = w;
    e->info.height = h;
    e->info.stride = of.fmt.pix_mp.plane_fmt[0].bytesperline;
    e->info.in_size = of.fmt.pix_mp.plane_fmt[0].sizeimage;
    // Высота Y-плоскости с выравниванием драйвера: Y + UV (вдвое ниже) = sizeimage.
    e->info.y_scanlines = e->info.stride ? (uint32_t)((uint64_t)e->info.in_size * 2 / 3 / e->info.stride) : h;
    if (e->info.y_scanlines < h) e->info.y_scanlines = h;

    struct v4l2_streamparm p = {.type = V4L2_BUF_TYPE_VIDEO_OUTPUT_MPLANE};
    p.parm.output.timeperframe.numerator = 1;
    p.parm.output.timeperframe.denominator = fps ? fps : 60;
    ioctl(fd, VIDIOC_S_PARM, &p);
    ctrl(fd, V4L2_CID_MPEG_VIDEO_BITRATE_MODE, V4L2_MPEG_VIDEO_BITRATE_MODE_VBR);
    ctrl(fd, V4L2_CID_MPEG_VIDEO_FRAME_RC_ENABLE, 1);
    ctrl(fd, V4L2_CID_MPEG_VIDEO_BITRATE, (int32_t)bitrate);
    ctrl(fd, V4L2_CID_MPEG_VIDEO_BITRATE_PEAK, (int32_t)(bitrate + bitrate / 2));
    // Ключевой кадр раз в gop кадров (перемотка в проигрывателях).
    ctrl(fd, V4L2_CID_MPEG_VIDEO_GOP_SIZE, (int32_t)gop);
    ctrl(fd, V4L2_CID_MPEG_VIDEO_B_FRAMES, 0);
    ctrl(fd, V4L2_CID_MPEG_VIDEO_PREPEND_SPSPPS_TO_IDR, 1);
    ctrl(fd, V4L2_CID_MPEG_VIDEO_HEADER_MODE, V4L2_MPEG_VIDEO_HEADER_MODE_JOINED_WITH_1ST_FRAME);
    if (hevc) {
        ctrl(fd, V4L2_CID_MPEG_VIDEO_HEVC_PROFILE, V4L2_MPEG_VIDEO_HEVC_PROFILE_MAIN);
    } else {
        ctrl(fd, V4L2_CID_MPEG_VIDEO_H264_PROFILE, V4L2_MPEG_VIDEO_H264_PROFILE_HIGH);
    }

    struct v4l2_requestbuffers rb = {.count = 4, .type = V4L2_BUF_TYPE_VIDEO_OUTPUT_MPLANE, .memory = V4L2_MEMORY_DMABUF};
    if (ioctl(fd, VIDIOC_REQBUFS, &rb) < 0) { seterr(err, errlen, "REQBUFS вход"); venc_close(e); return NULL; }
    e->info.in_count = rb.count > MAXBUF ? MAXBUF : rb.count;
    struct v4l2_requestbuffers rc = {.count = 4, .type = V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE, .memory = V4L2_MEMORY_DMABUF};
    if (ioctl(fd, VIDIOC_REQBUFS, &rc) < 0) { seterr(err, errlen, "REQBUFS выход"); venc_close(e); return NULL; }
    e->cap_count = rc.count > MAXBUF ? MAXBUF : rc.count;
    for (uint32_t i = 0; i < e->info.in_count; i++) {
        e->in_fd[i] = heap_alloc(e->info.in_size);
        if (e->in_fd[i] < 0) { seterr(err, errlen, "dma-heap (вход)"); venc_close(e); return NULL; }
    }
    e->cap_size = cf.fmt.pix_mp.plane_fmt[0].sizeimage;
    for (uint32_t i = 0; i < e->cap_count; i++) {
        e->cap_fd[i] = heap_alloc(e->cap_size);
        if (e->cap_fd[i] < 0) { seterr(err, errlen, "dma-heap (выход)"); venc_close(e); return NULL; }
        e->cap_map[i] = mmap(NULL, e->cap_size, PROT_READ, MAP_SHARED, e->cap_fd[i], 0);
        if (e->cap_map[i] == MAP_FAILED) { seterr(err, errlen, "mmap выхода"); venc_close(e); return NULL; }
        if (queue_cap(e, i) < 0) { seterr(err, errlen, "QBUF выход"); venc_close(e); return NULL; }
    }
    int t = V4L2_BUF_TYPE_VIDEO_OUTPUT_MPLANE;
    if (ioctl(fd, VIDIOC_STREAMON, &t) < 0) { seterr(err, errlen, "STREAMON вход"); venc_close(e); return NULL; }
    t = V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE;
    if (ioctl(fd, VIDIOC_STREAMON, &t) < 0) { seterr(err, errlen, "STREAMON выход"); venc_close(e); return NULL; }
    *info = e->info;
    return e;
}

int venc_in_fd(struct venc *e, uint32_t i) { return i < e->info.in_count ? e->in_fd[i] : -1; }

// Свободный входной буфер (кодер его уже вернул); -1 — нет.
int venc_free_input(struct venc *e) {
    for (;;) {
        struct v4l2_plane pl = {0};
        struct v4l2_buffer b = {.type = V4L2_BUF_TYPE_VIDEO_OUTPUT_MPLANE, .memory = V4L2_MEMORY_DMABUF, .length = 1, .m.planes = &pl};
        if (ioctl(e->fd, VIDIOC_DQBUF, &b) < 0) break;
        if (b.index < MAXBUF) e->in_busy[b.index] = 0;
    }
    for (uint32_t i = 0; i < e->info.in_count; i++)
        if (!e->in_busy[i]) return (int)i;
    return -1;
}

void venc_set_bitrate(struct venc *e, uint32_t bitrate) {
    ctrl(e->fd, V4L2_CID_MPEG_VIDEO_BITRATE, (int32_t)bitrate);
    ctrl(e->fd, V4L2_CID_MPEG_VIDEO_BITRATE_PEAK, (int32_t)(bitrate + bitrate / 2));
}

// Асинхронно: отдать входной буфер `i` кодеру (кадр в нём уже лежит), метка времени ts_us.
int venc_queue(struct venc *e, uint32_t i, int force_key, uint64_t ts_us) {
    if (force_key) ctrl(e->fd, V4L2_CID_MPEG_VIDEO_FORCE_KEY_FRAME, 1);
    struct v4l2_plane pl = {.m.fd = e->in_fd[i], .length = e->info.in_size, .bytesused = e->info.in_size};
    struct v4l2_buffer b = {.index = i, .type = V4L2_BUF_TYPE_VIDEO_OUTPUT_MPLANE, .memory = V4L2_MEMORY_DMABUF,
                            .length = 1, .m.planes = &pl};
    b.timestamp.tv_sec = ts_us / 1000000;
    b.timestamp.tv_usec = ts_us % 1000000;
    if (ioctl(e->fd, VIDIOC_QBUF, &b) < 0) return -errno;
    e->in_busy[i] = 1;
    return 0;
}

// Асинхронно: один готовый буфер выхода (ждать не дольше timeout_ms). Ответ — длина (данные в *out до
// следующего вызова), 0 — пусто, <0 — ошибка (-errno); *ts_us — метка кадра, *key — ключевой.
long venc_dequeue(struct venc *e, int timeout_ms, const uint8_t **out, uint64_t *ts_us, int *key) {
    for (;;) {
        struct v4l2_plane cp = {0};
        struct v4l2_buffer cb = {.type = V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE, .memory = V4L2_MEMORY_DMABUF, .length = 1, .m.planes = &cp};
        if (ioctl(e->fd, VIDIOC_DQBUF, &cb) < 0) {
            if (errno != EAGAIN) return -errno;
            if (timeout_ms <= 0) return 0;
            struct pollfd pfd = {.fd = e->fd, .events = POLLIN};
            int r = poll(&pfd, 1, timeout_ms);
            if (r < 0 && errno != EINTR) return -errno;
            if (r == 0) return 0;
            timeout_ms = 0;  // одно ожидание; дальше — что есть
            continue;
        }
        uint32_t n = cp.bytesused > cp.data_offset ? cp.bytesused - cp.data_offset : 0;
        if (n > e->out_cap) {
            e->out_cap = n * 2;
            e->out = realloc(e->out, e->out_cap);
        }
        memcpy(e->out, e->cap_map[cb.index] + cp.data_offset, n);
        *key = !!(cb.flags & V4L2_BUF_FLAG_KEYFRAME);
        *ts_us = (uint64_t)cb.timestamp.tv_sec * 1000000 + cb.timestamp.tv_usec;
        queue_cap(e, cb.index);
        *out = e->out;
        return (long)n;
    }
}

// Закодировать входной буфер `i` (кадр в нём уже нарисован). Ответ — длина пакета
// (данные в *out до следующего вызова), <0 — ошибка (-errno).
long venc_encode(struct venc *e, uint32_t i, int force_key, uint64_t ts_us, int timeout_ms, const uint8_t **out, int *key) {
    if (force_key) ctrl(e->fd, V4L2_CID_MPEG_VIDEO_FORCE_KEY_FRAME, 1);
    struct v4l2_plane pl = {.m.fd = e->in_fd[i], .length = e->info.in_size, .bytesused = e->info.in_size};
    struct v4l2_buffer b = {.index = i, .type = V4L2_BUF_TYPE_VIDEO_OUTPUT_MPLANE, .memory = V4L2_MEMORY_DMABUF,
                            .length = 1, .m.planes = &pl};
    b.timestamp.tv_sec = ts_us / 1000000;
    b.timestamp.tv_usec = ts_us % 1000000;
    if (ioctl(e->fd, VIDIOC_QBUF, &b) < 0) return -errno;
    e->in_busy[i] = 1;
    size_t len = 0;
    int got = 0;
    // Пакет одного кадра может прийти несколькими буферами (заголовки отдельно) —
    // собираем, пока не придёт буфер с меткой времени этого кадра.
    for (;;) {
        struct pollfd pfd = {.fd = e->fd, .events = POLLIN};
        int r = poll(&pfd, 1, timeout_ms);
        if (r == 0) return got ? (long)len : -ETIMEDOUT;
        if (r < 0) {
            if (errno == EINTR) continue;
            return -errno;
        }
        struct v4l2_plane cp = {0};
        struct v4l2_buffer cb = {.type = V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE, .memory = V4L2_MEMORY_DMABUF, .length = 1, .m.planes = &cp};
        if (ioctl(e->fd, VIDIOC_DQBUF, &cb) < 0) {
            if (errno == EAGAIN) continue;
            return -errno;
        }
        uint32_t n = cp.bytesused > cp.data_offset ? cp.bytesused - cp.data_offset : 0;
        if (len + n > e->out_cap) {
            e->out_cap = (len + n) * 2;
            e->out = realloc(e->out, e->out_cap);
        }
        memcpy(e->out + len, e->cap_map[cb.index] + cp.data_offset, n);
        len += n;
        if (cb.flags & V4L2_BUF_FLAG_KEYFRAME) *key = 1;
        uint64_t ts = (uint64_t)cb.timestamp.tv_sec * 1000000 + cb.timestamp.tv_usec;
        queue_cap(e, cb.index);
        got = 1;
        if (ts == ts_us && n > 0) break;
    }
    *out = e->out;
    return (long)len;
}
