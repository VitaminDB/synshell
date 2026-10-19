/*
 * Эмуляция System V IPC (семафоры и разделяемая память) для ядер без
 * CONFIG_SYSVIPC (Android). Подгружается через LD_PRELOAD.
 *
 * Объекты — файлы в /dev/shm/.sysvipc: sem.<id>, shm.<id>.
 * Для ключа != IPC_PRIVATE id выводится из ключа, поэтому разные процессы
 * находят один и тот же объект. Ожидание — futex на счётчике изменений.
 * SEM_UNDO игнорируется.
 */
#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <linux/futex.h>
#include <pthread.h>
#include <stdarg.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ipc.h>
#include <sys/mman.h>
#include <sys/sem.h>
#include <sys/shm.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <sys/time.h>
#include <time.h>
#include <unistd.h>

#define DIR_PATH "/dev/shm/.sysvipc"
#define SEM_MAGIC 0x53454d31u /* "SEM1" */
#define SEM_MAX 512

union semun_ {
    int val;
    struct semid_ds *buf;
    unsigned short *array;
};

struct sem_set {
    uint32_t magic;
    uint32_t nsems;
    uint32_t lock;   /* 0 — свободно, 1 — занято */
    uint32_t seq;    /* растёт при каждом изменении, на нём ждут */
    int32_t mode;
    int32_t pad;
    int32_t val[];
};

static int ensure_dir(void)
{
    if (mkdir(DIR_PATH, 01777)) { if (errno != EEXIST) return -1; }
    else chmod(DIR_PATH, 01777);
    return 0;
}

static void obj_path(char *buf, size_t len, const char *kind, int id)
{
    snprintf(buf, len, DIR_PATH "/%s.%d", kind, id);
}

static int key_to_id(key_t key)
{
    int id = (int)((uint32_t)key & 0x3fffffffu);
    return id ? id : 1;
}

/* открыть/создать объект; для IPC_PRIVATE подбирается свободный id */
static int obj_open(const char *kind, key_t key, int flags, int *id_out, int *created)
{
    char path[PATH_MAX];
    int fd, id, oflags = O_RDWR | O_CLOEXEC, mode = flags & 0777;

    if (ensure_dir()) return -1;
    *created = 0;

    if (key == IPC_PRIVATE)
    {
        for (int tries = 0; tries < 1000; tries++)
        {
            id = 0x40000000 | (int)(((uint32_t)rand() ^ ((uint32_t)getpid() << 12) ^ (uint32_t)tries) & 0x3fffffff);
            obj_path(path, sizeof(path), kind, id);
            fd = open(path, oflags | O_CREAT | O_EXCL, mode ? mode : 0600);
            if (fd >= 0) { *id_out = id; *created = 1; return fd; }
            if (errno != EEXIST) return -1;
        }
        errno = ENOSPC;
        return -1;
    }

    id = key_to_id(key);
    obj_path(path, sizeof(path), kind, id);
    if (flags & IPC_CREAT)
    {
        fd = open(path, oflags | O_CREAT | O_EXCL, mode ? mode : 0600);
        if (fd >= 0) { *id_out = id; *created = 1; return fd; }
        if (errno != EEXIST) return -1;
        if (flags & IPC_EXCL) { errno = EEXIST; return -1; }
    }
    fd = open(path, oflags);
    if (fd < 0 && errno == ENOENT) errno = ENOENT;
    if (fd >= 0) *id_out = id;
    return fd;
}

static int futex(uint32_t *addr, int op, uint32_t val, const struct timespec *ts)
{
    return syscall(SYS_futex, addr, op, val, ts, NULL, 0);
}

/* ---------------------------------------------------------------- семафоры */

struct sem_map { int id; struct sem_set *set; size_t size; };
static struct sem_map sem_maps[SEM_MAX];
static int sem_nmaps;
static pthread_mutex_t sem_maps_lock = PTHREAD_MUTEX_INITIALIZER;

static void set_lock(struct sem_set *s)
{
    while (__atomic_exchange_n(&s->lock, 1, __ATOMIC_ACQUIRE))
        futex(&s->lock, FUTEX_WAIT, 1, NULL);
}

static void set_unlock(struct sem_set *s)
{
    __atomic_store_n(&s->lock, 0, __ATOMIC_RELEASE);
    futex(&s->lock, FUTEX_WAKE, 1, NULL);
}

static struct sem_set *sem_lookup(int id)
{
    char path[PATH_MAX];
    struct sem_set *set = NULL;
    struct stat st;
    int fd;

    pthread_mutex_lock(&sem_maps_lock);
    for (int i = 0; i < sem_nmaps; i++)
        if (sem_maps[i].id == id) { set = sem_maps[i].set; goto done; }

    obj_path(path, sizeof(path), "sem", id);
    if ((fd = open(path, O_RDWR | O_CLOEXEC)) < 0) { errno = EINVAL; goto done; }
    if (fstat(fd, &st) || (size_t)st.st_size < sizeof(struct sem_set)) { close(fd); errno = EINVAL; goto done; }
    set = mmap(NULL, st.st_size, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    close(fd);
    if (set == MAP_FAILED || set->magic != SEM_MAGIC)
    {
        if (set != MAP_FAILED) munmap(set, st.st_size);
        set = NULL;
        errno = EINVAL;
        goto done;
    }
    if (sem_nmaps < SEM_MAX)
    {
        sem_maps[sem_nmaps].id = id;
        sem_maps[sem_nmaps].set = set;
        sem_maps[sem_nmaps].size = st.st_size;
        sem_nmaps++;
    }
done:
    pthread_mutex_unlock(&sem_maps_lock);
    return set;
}

static void sem_forget(int id)
{
    pthread_mutex_lock(&sem_maps_lock);
    for (int i = 0; i < sem_nmaps; i++)
        if (sem_maps[i].id == id)
        {
            munmap(sem_maps[i].set, sem_maps[i].size);
            sem_maps[i] = sem_maps[--sem_nmaps];
            break;
        }
    pthread_mutex_unlock(&sem_maps_lock);
}

int semget(key_t key, int nsems, int semflg)
{
    int fd, id, created;
    size_t size;
    struct sem_set *set;

    if (nsems < 0 || nsems > 32000) { errno = EINVAL; return -1; }
    if ((fd = obj_open("sem", key, semflg, &id, &created)) < 0) return -1;

    if (created)
    {
        if (!nsems) nsems = 1;
        size = sizeof(struct sem_set) + nsems * sizeof(int32_t);
        if (ftruncate(fd, size)) { close(fd); return -1; }
        set = mmap(NULL, size, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
        if (set == MAP_FAILED) { close(fd); return -1; }
        set->nsems = nsems;
        set->mode = semflg & 0777;
        __atomic_store_n(&set->magic, SEM_MAGIC, __ATOMIC_RELEASE);
        munmap(set, size);
    }
    else
    {
        /* дождаться инициализации, если объект только что создан другим процессом */
        struct stat st;
        for (int i = 0; i < 1000; i++)
        {
            if (!fstat(fd, &st) && (size_t)st.st_size >= sizeof(struct sem_set)) break;
            usleep(1000);
        }
        if (nsems && (set = sem_lookup(id)) && set->nsems < (uint32_t)nsems)
        {
            close(fd);
            errno = EINVAL;
            return -1;
        }
    }
    close(fd);
    return id;
}

static int do_semop(int semid, struct sembuf *sops, size_t nsops, const struct timespec *timeout)
{
    struct sem_set *set = sem_lookup(semid);
    struct timespec deadline, now, rel;

    if (!set) return -1;
    if (!nsops) { errno = EINVAL; return -1; }
    for (size_t i = 0; i < nsops; i++)
        if (sops[i].sem_num >= set->nsems) { errno = EFBIG; return -1; }

    if (timeout)
    {
        clock_gettime(CLOCK_MONOTONIC, &deadline);
        deadline.tv_sec += timeout->tv_sec;
        deadline.tv_nsec += timeout->tv_nsec;
        if (deadline.tv_nsec >= 1000000000) { deadline.tv_sec++; deadline.tv_nsec -= 1000000000; }
    }

    for (;;)
    {
        int ok = 1, nowait = 0;
        uint32_t seq;

        set_lock(set);
        for (size_t i = 0; i < nsops && ok; i++)
        {
            int32_t v = set->val[sops[i].sem_num];
            /* учесть предыдущие операции над тем же семафором */
            for (size_t j = 0; j < i; j++)
                if (sops[j].sem_num == sops[i].sem_num) v += sops[j].sem_op;
            if (sops[i].sem_op < 0 && v + sops[i].sem_op < 0) ok = 0;
            if (sops[i].sem_op == 0 && v != 0) ok = 0;
            if (!ok && (sops[i].sem_flg & IPC_NOWAIT)) nowait = 1;
        }
        if (ok)
        {
            for (size_t i = 0; i < nsops; i++) set->val[sops[i].sem_num] += sops[i].sem_op;
            __atomic_add_fetch(&set->seq, 1, __ATOMIC_RELEASE);
            set_unlock(set);
            futex(&set->seq, FUTEX_WAKE, INT_MAX, NULL);
            return 0;
        }
        seq = __atomic_load_n(&set->seq, __ATOMIC_ACQUIRE);
        set_unlock(set);
        if (nowait) { errno = EAGAIN; return -1; }

        if (timeout)
        {
            clock_gettime(CLOCK_MONOTONIC, &now);
            rel.tv_sec = deadline.tv_sec - now.tv_sec;
            rel.tv_nsec = deadline.tv_nsec - now.tv_nsec;
            if (rel.tv_nsec < 0) { rel.tv_sec--; rel.tv_nsec += 1000000000; }
            if (rel.tv_sec < 0) { errno = EAGAIN; return -1; }
            if (futex(&set->seq, FUTEX_WAIT, seq, &rel) && errno == ETIMEDOUT) { errno = EAGAIN; return -1; }
        }
        else futex(&set->seq, FUTEX_WAIT, seq, NULL);

        if (errno == EINTR) { errno = EINTR; return -1; }
    }
}

int semop(int semid, struct sembuf *sops, size_t nsops)
{
    return do_semop(semid, sops, nsops, NULL);
}

int semtimedop(int semid, struct sembuf *sops, size_t nsops, const struct timespec *timeout)
{
    return do_semop(semid, sops, nsops, timeout);
}

int semctl(int semid, int semnum, int cmd, ...)
{
    union semun_ arg = {0};
    struct sem_set *set;
    va_list ap;
    int ret = 0;

    va_start(ap, cmd);
    switch (cmd & 0xff)
    {
    case SETVAL: arg.val = va_arg(ap, int); break;
    case GETALL: case SETALL: arg.array = va_arg(ap, unsigned short *); break;
    case IPC_STAT: case IPC_SET: case SEM_STAT: arg.buf = va_arg(ap, struct semid_ds *); break;
    default: break;
    }
    va_end(ap);
    cmd &= 0xff;

    if (cmd == IPC_RMID)
    {
        char path[PATH_MAX];
        if ((set = sem_lookup(semid)))
        {
            set_lock(set);
            set->magic = 0;
            __atomic_add_fetch(&set->seq, 1, __ATOMIC_RELEASE);
            set_unlock(set);
            futex(&set->seq, FUTEX_WAKE, INT_MAX, NULL);
        }
        obj_path(path, sizeof(path), "sem", semid);
        sem_forget(semid);
        return unlink(path) && errno != ENOENT ? -1 : 0;
    }

    if (!(set = sem_lookup(semid))) return -1;
    if ((cmd == GETVAL || cmd == SETVAL || cmd == GETPID || cmd == GETNCNT || cmd == GETZCNT)
        && (semnum < 0 || (uint32_t)semnum >= set->nsems))
    {
        errno = EINVAL;
        return -1;
    }

    set_lock(set);
    switch (cmd)
    {
    case GETVAL: ret = set->val[semnum]; break;
    case SETVAL: set->val[semnum] = arg.val; __atomic_add_fetch(&set->seq, 1, __ATOMIC_RELEASE); break;
    case GETALL:
        for (uint32_t i = 0; i < set->nsems; i++) arg.array[i] = set->val[i];
        break;
    case SETALL:
        for (uint32_t i = 0; i < set->nsems; i++) set->val[i] = arg.array[i];
        __atomic_add_fetch(&set->seq, 1, __ATOMIC_RELEASE);
        break;
    case GETPID: case GETNCNT: case GETZCNT: ret = 0; break;
    case IPC_STAT: case SEM_STAT:
        memset(arg.buf, 0, sizeof(*arg.buf));
        arg.buf->sem_nsems = set->nsems;
        arg.buf->sem_perm.mode = set->mode;
        arg.buf->sem_perm.uid = arg.buf->sem_perm.cuid = getuid();
        arg.buf->sem_perm.gid = arg.buf->sem_perm.cgid = getgid();
        if (cmd == SEM_STAT) ret = semid;
        break;
    case IPC_SET: break;
    default: errno = EINVAL; ret = -1; break;
    }
    set_unlock(set);
    if (cmd == SETVAL || cmd == SETALL) futex(&set->seq, FUTEX_WAKE, INT_MAX, NULL);
    return ret;
}

/* ------------------------------------------------------------ разделяемая память */

#define SHM_MAX 1024
struct shm_att { void *addr; size_t size; int id; };
static struct shm_att shm_atts[SHM_MAX];
static pthread_mutex_t shm_lock = PTHREAD_MUTEX_INITIALIZER;

int shmget(key_t key, size_t size, int shmflg)
{
    int fd, id, created;
    struct stat st;

    if ((fd = obj_open("shm", key, shmflg, &id, &created)) < 0) return -1;
    if (created)
    {
        if (!size || ftruncate(fd, size)) { int e = size ? errno : EINVAL; close(fd); errno = e; return -1; }
    }
    else if (!fstat(fd, &st) && size > (size_t)st.st_size)
    {
        close(fd);
        errno = EINVAL;
        return -1;
    }
    close(fd);
    return id;
}

void *shmat(int shmid, const void *shmaddr, int shmflg)
{
    char path[PATH_MAX];
    struct stat st;
    void *addr;
    int fd, rdonly = shmflg & SHM_RDONLY;

    obj_path(path, sizeof(path), "shm", shmid);
    if ((fd = open(path, (rdonly ? O_RDONLY : O_RDWR) | O_CLOEXEC)) < 0) { errno = EINVAL; return (void *)-1; }
    if (fstat(fd, &st)) { close(fd); return (void *)-1; }
    addr = mmap((void *)shmaddr, st.st_size, PROT_READ | (rdonly ? 0 : PROT_WRITE),
                MAP_SHARED | (shmaddr ? MAP_FIXED : 0), fd, 0);
    close(fd);
    if (addr == MAP_FAILED) return (void *)-1;

    pthread_mutex_lock(&shm_lock);
    for (int i = 0; i < SHM_MAX; i++)
        if (!shm_atts[i].addr)
        {
            shm_atts[i].addr = addr;
            shm_atts[i].size = st.st_size;
            shm_atts[i].id = shmid;
            break;
        }
    pthread_mutex_unlock(&shm_lock);
    return addr;
}

int shmdt(const void *shmaddr)
{
    int ret = -1;

    pthread_mutex_lock(&shm_lock);
    for (int i = 0; i < SHM_MAX; i++)
        if (shm_atts[i].addr == shmaddr)
        {
            ret = munmap(shm_atts[i].addr, shm_atts[i].size);
            shm_atts[i].addr = NULL;
            break;
        }
    pthread_mutex_unlock(&shm_lock);
    if (ret < 0 && !errno) errno = EINVAL;
    return ret;
}

int shmctl(int shmid, int cmd, struct shmid_ds *buf)
{
    char path[PATH_MAX];
    struct stat st;

    obj_path(path, sizeof(path), "shm", shmid);
    switch (cmd & 0xff)
    {
    case IPC_RMID:
        /* уже подключённые отображения остаются валидными, как в Linux */
        return unlink(path) && errno != ENOENT ? -1 : 0;
    case IPC_STAT:
    case SHM_STAT:
        if (stat(path, &st)) { errno = EINVAL; return -1; }
        memset(buf, 0, sizeof(*buf));
        buf->shm_segsz = st.st_size;
        buf->shm_nattch = 1;
        buf->shm_perm.mode = st.st_mode & 0777;
        buf->shm_perm.uid = buf->shm_perm.cuid = st.st_uid;
        buf->shm_perm.gid = buf->shm_perm.cgid = st.st_gid;
        buf->shm_cpid = buf->shm_lpid = getpid();
        return (cmd & 0xff) == SHM_STAT ? shmid : 0;
    case IPC_SET:
    case SHM_LOCK:
    case SHM_UNLOCK:
        return 0;
    default:
        errno = EINVAL;
        return -1;
    }
}
