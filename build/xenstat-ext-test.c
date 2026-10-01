/*
 * xenstat-ext-test: sanity check for the xentop-ng libxenstat extensions.
 *
 * Takes two libxenstat samples 1 s apart and prints, for every domain/VBD,
 * the extended VBD3 counters (plus per-second deltas and mean latency), and
 * for every pCPU the cumulative idle time and derived busy percentage.
 *
 * Run as root in dom0:  LD_LIBRARY_PATH=/opt/xentop-ng/lib ./xenstat-ext-test
 * (built with RUNPATH=/opt/xentop-ng/lib, so LD_LIBRARY_PATH is optional).
 */
#include <stdio.h>
#include <stdlib.h>
#include <time.h>
#include <unistd.h>
#include <xenstat.h>

static double now_s(void)
{
    struct timespec ts;

    clock_gettime(CLOCK_MONOTONIC, &ts);
    return ts.tv_sec + ts.tv_nsec / 1e9;
}

static const char *vbd_type(unsigned int t)
{
    switch (t) {
    case 0: return "qdisk/other";
    case 1: return "vbd";       /* blkback */
    case 2: return "tap";
    case 3: return "vbd3";      /* tapdisk3 */
    default: return "?";
    }
}

static xenstat_vbd *find_vbd(xenstat_node *node, unsigned int domid,
                             unsigned int type, unsigned int dev)
{
    xenstat_domain *d = xenstat_node_domain(node, domid);
    unsigned int i;

    if (!d)
        return NULL;
    for (i = 0; i < xenstat_domain_num_vbds(d); i++) {
        xenstat_vbd *v = xenstat_domain_vbd(d, i);

        if (xenstat_vbd_type(v) == type && xenstat_vbd_dev(v) == dev)
            return v;
    }
    return NULL;
}

static double avg_ms(unsigned long long dus, unsigned long long dreq)
{
    return dreq ? (double)dus / dreq / 1000.0 : 0.0;
}

/* VM names are set by toolstack users who may be less privileged than us:
 * print them with control and non-ASCII bytes replaced, so a name cannot
 * smuggle terminal escape sequences into root's terminal. */
static const char *safe_name(const char *s, char *buf, size_t len)
{
    size_t i;

    if (!s)
        return "(null)";
    for (i = 0; s[i] && i + 1 < len; i++)
        buf[i] = (s[i] >= 0x20 && s[i] < 0x7f) ? s[i] : '?';
    buf[i] = '\0';
    return buf;
}

int main(void)
{
    xenstat_handle *h;
    xenstat_node *n0, *n1;
    double t0, t1, dt;
    unsigned int i, j, ncpu;

    h = xenstat_init();
    if (!h) {
        perror("xenstat_init (are you root in dom0?)");
        return 1;
    }

    t0 = now_s();
    n0 = xenstat_get_node(h, XENSTAT_ALL);
    sleep(1);
    t1 = now_s();
    n1 = xenstat_get_node(h, XENSTAT_ALL);
    if (!n0 || !n1) {
        perror("xenstat_get_node");
        return 1;
    }
    dt = t1 - t0;

    printf("interval %.3f s, num_cpus %u, num_pcpu_idle %u\n\n",
           dt, xenstat_node_num_cpus(n1), xenstat_node_num_pcpu_idle(n1));

    printf("== pCPU idle ==\n%-5s %20s %20s %8s\n",
           "cpu", "idle_ns[t0]", "idle_ns[t1]", "busy%");
    ncpu = xenstat_node_num_pcpu_idle(n1);
    if (ncpu == 0)
        ncpu = xenstat_node_num_cpus(n1);
    for (i = 0; i < ncpu; i++) {
        unsigned long long a = xenstat_node_pcpu_idle_ns(n0, i);
        unsigned long long b = xenstat_node_pcpu_idle_ns(n1, i);
        double idle = (b >= a) ? (double)(b - a) / (dt * 1e9) : 0.0;

        printf("%-5u %20llu %20llu %7.1f%%\n", i, a, b,
               b ? 100.0 * (1.0 - (idle > 1.0 ? 1.0 : idle)) : 0.0);
    }

    printf("\n== VBDs ==\n");
    char name[128];

    for (i = 0; i < xenstat_node_num_domains(n1); i++) {
        xenstat_domain *d = xenstat_node_domain_by_index(n1, i);
        unsigned int domid = xenstat_domain_id(d);

        for (j = 0; j < xenstat_domain_num_vbds(d); j++) {
            xenstat_vbd *v = xenstat_domain_vbd(d, j);
            xenstat_vbd *p = find_vbd(n0, domid, xenstat_vbd_type(v),
                                      xenstat_vbd_dev(v));

            printf("dom %u (%s) %s-%u err=%d has_ext=%u\n"
                   "  submitted rd=%llu wr=%llu  sects rd=%llu wr=%llu  oo=%llu\n",
                   domid, safe_name(xenstat_domain_name(d), name, sizeof(name)),
                   vbd_type(xenstat_vbd_type(v)), xenstat_vbd_dev(v),
                   xenstat_vbd_error(v), xenstat_vbd_has_ext(v),
                   xenstat_vbd_rd_reqs(v), xenstat_vbd_wr_reqs(v),
                   xenstat_vbd_rd_sects(v), xenstat_vbd_wr_sects(v),
                   xenstat_vbd_oo_reqs(v));
            if (!xenstat_vbd_has_ext(v))
                continue;
            printf("  done rd=%llu wr=%llu  usecs rd=%llu wr=%llu  io_errors=%llu\n",
                   xenstat_vbd_rd_reqs_done(v), xenstat_vbd_wr_reqs_done(v),
                   xenstat_vbd_rd_usecs(v), xenstat_vbd_wr_usecs(v),
                   xenstat_vbd_io_errors(v));
            if (p && xenstat_vbd_has_ext(p)) {
                unsigned long long drd = xenstat_vbd_rd_reqs_done(v) -
                                         xenstat_vbd_rd_reqs_done(p);
                unsigned long long dwr = xenstat_vbd_wr_reqs_done(v) -
                                         xenstat_vbd_wr_reqs_done(p);

                printf("  delta: rd %.0f/s avg %.3f ms, wr %.0f/s avg %.3f ms\n",
                       drd / dt,
                       avg_ms(xenstat_vbd_rd_usecs(v) - xenstat_vbd_rd_usecs(p), drd),
                       dwr / dt,
                       avg_ms(xenstat_vbd_wr_usecs(v) - xenstat_vbd_wr_usecs(p), dwr));
            }
        }
    }

    xenstat_free_node(n0);
    xenstat_free_node(n1);
    xenstat_uninit(h);
    return 0;
}
