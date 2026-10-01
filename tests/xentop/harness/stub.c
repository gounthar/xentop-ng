/*
 * Test harness: a fake libxenstat (plus the pcpu.h helpers, the clock and
 * sleep) that replays a fixture file, so the real xentop.c can be compiled
 * against it and its exact output recorded as golden data for xentop-ng's
 * compatibility mode. See regen.sh.
 *
 * Fixture format (one record per line, '#' comments):
 *   node <t_us> <tot_mem> <free_mem> <num_cpus> <cpu_hz> <xen_version>
 *   pcpu <idle_ns>
 *   dom <id> <flags dsbcpr or -> <cpu_ns> <cur_mem> <max_mem> <ssid> <name...>
 *   vcpu <online> <ns>
 *   net <id> <rbytes> <rpkts> <rerrs> <rdrop> <tbytes> <tpkts> <terrs> <tdrop>
 *   vbd <type> <dev> <error> <oo> <rd> <wr> <rsect> <wsect>
 * Each "node" line starts the sample returned by the next xenstat_get_node().
 *
 * SPDX-License-Identifier: GPL-2.0-only
 */
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/time.h>
#include <xenstat.h>
#include "pcpu.h"

#define MAXN 8
#define MAXD 64
#define MAXV 64
#define MAXP 256

struct xenstat_vcpu { unsigned online; unsigned long long ns; };
struct xenstat_network {
	unsigned id;
	unsigned long long rb, rp, re, rd, tb, tp, te, td;
};
struct xenstat_vbd {
	unsigned type, dev; bool err;
	unsigned long long oo, rd, wr, rs, ws;
};
struct xenstat_domain {
	unsigned id, ssid; char flags[8]; char name[256];
	unsigned long long cpu_ns, cur_mem, max_mem;
	unsigned nv, nn, nb;
	struct xenstat_vcpu v[MAXV];
	struct xenstat_network n[MAXV];
	struct xenstat_vbd b[MAXV];
};
struct xenstat_node {
	unsigned long long t_us, tot, free_, hz;
	unsigned ncpus, nd, np;
	char ver[64];
	unsigned long long idle[MAXP];
	struct xenstat_domain d[MAXD];
};
struct xenstat_handle { int dummy; };

static struct xenstat_node nodes[MAXN];
static unsigned num_nodes, next_node;
static struct xenstat_handle the_handle;

static void die(const char *m) { fprintf(stderr, "stub: %s\n", m); exit(2); }

static void load(void)
{
	const char *path = getenv("XT_FIXTURE");
	char line[1024];
	FILE *f;
	struct xenstat_node *n = NULL;
	struct xenstat_domain *d = NULL;

	if (!path || !(f = fopen(path, "r")))
		die("set XT_FIXTURE");
	while (fgets(line, sizeof(line), f)) {
		char *p = line;
		int off = 0;
		line[strcspn(line, "\n")] = 0;
		if (!strncmp(p, "node ", 5)) {
			if (num_nodes == MAXN) die("too many nodes");
			n = &nodes[num_nodes++];
			d = NULL;
			if (sscanf(p, "node %llu %llu %llu %u %llu %63s", &n->t_us, &n->tot,
				   &n->free_, &n->ncpus, &n->hz, n->ver) != 6)
				die(line);
		} else if (!strncmp(p, "pcpu ", 5)) {
			if (sscanf(p, "pcpu %llu", &n->idle[n->np++]) != 1) die(line);
		} else if (!strncmp(p, "dom ", 4)) {
			d = &n->d[n->nd++];
			if (sscanf(p, "dom %u %7s %llu %llu %llu %u %n", &d->id, d->flags,
				   &d->cpu_ns, &d->cur_mem, &d->max_mem, &d->ssid, &off) != 6)
				die(line);
			snprintf(d->name, sizeof(d->name), "%s", p + off);
		} else if (!strncmp(p, "vcpu ", 5)) {
			struct xenstat_vcpu *v = &d->v[d->nv++];
			if (sscanf(p, "vcpu %u %llu", &v->online, &v->ns) != 2) die(line);
		} else if (!strncmp(p, "net ", 4)) {
			struct xenstat_network *x = &d->n[d->nn++];
			if (sscanf(p, "net %u %llu %llu %llu %llu %llu %llu %llu %llu", &x->id,
				   &x->rb, &x->rp, &x->re, &x->rd, &x->tb, &x->tp, &x->te,
				   &x->td) != 9)
				die(line);
		} else if (!strncmp(p, "vbd ", 4)) {
			struct xenstat_vbd *x = &d->b[d->nb++];
			unsigned e;
			if (sscanf(p, "vbd %u %u %u %llu %llu %llu %llu %llu", &x->type, &x->dev,
				   &e, &x->oo, &x->rd, &x->wr, &x->rs, &x->ws) != 8)
				die(line);
			x->err = e;
		}
	}
	fclose(f);
}

/* Clock and sleep: the time of the sample about to be taken. */
int gettimeofday(struct timeval *tv, void *tz)
{
	unsigned long long t = nodes[next_node < num_nodes ? next_node : num_nodes - 1].t_us;
	(void)tz;
	tv->tv_sec = t / 1000000;
	tv->tv_usec = t % 1000000;
	return 0;
}
unsigned int sleep(unsigned int s) { (void)s; return 0; }

xenstat_handle *xenstat_init(void) { load(); return &the_handle; }
void xenstat_uninit(xenstat_handle *h) { (void)h; }
xenstat_node *xenstat_get_node(xenstat_handle *h, unsigned int flags)
{
	(void)h; (void)flags;
	return next_node < num_nodes ? &nodes[next_node++] : NULL;
}
void xenstat_free_node(xenstat_node *n) { (void)n; }
xenstat_domain *xenstat_node_domain(xenstat_node *n, unsigned int id)
{
	for (unsigned i = 0; i < n->nd; i++)
		if (n->d[i].id == id) return &n->d[i];
	return NULL;
}
xenstat_domain *xenstat_node_domain_by_index(xenstat_node *n, unsigned int i) { return &n->d[i]; }
const char *xenstat_node_xen_version(xenstat_node *n) { return n->ver; }
unsigned long long xenstat_node_tot_mem(xenstat_node *n) { return n->tot; }
unsigned long long xenstat_node_free_mem(xenstat_node *n) { return n->free_; }
unsigned int xenstat_node_num_domains(xenstat_node *n) { return n->nd; }
unsigned int xenstat_node_num_cpus(xenstat_node *n) { return n->ncpus; }
unsigned long long xenstat_node_cpu_hz(xenstat_node *n) { return n->hz; }

unsigned xenstat_domain_id(xenstat_domain *d) { return d->id; }
char *xenstat_domain_name(xenstat_domain *d) { return d->name; }
unsigned long long xenstat_domain_cpu_ns(xenstat_domain *d) { return d->cpu_ns; }
unsigned int xenstat_domain_num_vcpus(xenstat_domain *d) { return d->nv; }
xenstat_vcpu *xenstat_domain_vcpu(xenstat_domain *d, unsigned int i) { return &d->v[i]; }
unsigned long long xenstat_domain_cur_mem(xenstat_domain *d) { return d->cur_mem; }
unsigned long long xenstat_domain_max_mem(xenstat_domain *d) { return d->max_mem; }
unsigned int xenstat_domain_ssid(xenstat_domain *d) { return d->ssid; }
static unsigned flag(xenstat_domain *d, char c) { return strchr(d->flags, c) != NULL; }
unsigned int xenstat_domain_dying(xenstat_domain *d) { return flag(d, 'd'); }
unsigned int xenstat_domain_crashed(xenstat_domain *d) { return flag(d, 'c'); }
unsigned int xenstat_domain_shutdown(xenstat_domain *d) { return flag(d, 's'); }
unsigned int xenstat_domain_paused(xenstat_domain *d) { return flag(d, 'p'); }
unsigned int xenstat_domain_blocked(xenstat_domain *d) { return flag(d, 'b'); }
unsigned int xenstat_domain_running(xenstat_domain *d) { return flag(d, 'r'); }
unsigned int xenstat_domain_num_networks(xenstat_domain *d) { return d->nn; }
xenstat_network *xenstat_domain_network(xenstat_domain *d, unsigned int i) { return &d->n[i]; }
unsigned int xenstat_domain_num_vbds(xenstat_domain *d) { return d->nb; }
xenstat_vbd *xenstat_domain_vbd(xenstat_domain *d, unsigned int i) { return &d->b[i]; }

unsigned int xenstat_vcpu_online(xenstat_vcpu *v) { return v->online; }
unsigned long long xenstat_vcpu_ns(xenstat_vcpu *v) { return v->ns; }

unsigned long long xenstat_network_rbytes(xenstat_network *x) { return x->rb; }
unsigned long long xenstat_network_rpackets(xenstat_network *x) { return x->rp; }
unsigned long long xenstat_network_rerrs(xenstat_network *x) { return x->re; }
unsigned long long xenstat_network_rdrop(xenstat_network *x) { return x->rd; }
unsigned long long xenstat_network_tbytes(xenstat_network *x) { return x->tb; }
unsigned long long xenstat_network_tpackets(xenstat_network *x) { return x->tp; }
unsigned long long xenstat_network_terrs(xenstat_network *x) { return x->te; }
unsigned long long xenstat_network_tdrop(xenstat_network *x) { return x->td; }

unsigned int xenstat_vbd_type(xenstat_vbd *x) { return x->type; }
unsigned int xenstat_vbd_dev(xenstat_vbd *x) { return x->dev; }
bool xenstat_vbd_error(xenstat_vbd *x) { return x->err; }
unsigned long long xenstat_vbd_oo_reqs(xenstat_vbd *x) { return x->oo; }
unsigned long long xenstat_vbd_rd_reqs(xenstat_vbd *x) { return x->rd; }
unsigned long long xenstat_vbd_wr_reqs(xenstat_vbd *x) { return x->wr; }
unsigned long long xenstat_vbd_rd_sects(xenstat_vbd *x) { return x->rs; }
unsigned long long xenstat_vbd_wr_sects(xenstat_vbd *x) { return x->ws; }

/*
 * pcpu.h, same arithmetic as tools/xentop/pcpu.c with xc_getcpuinfo()
 * replaced by the fixture's idle times of the sample just taken.
 */
static float usage[MAXP];
static unsigned long long prev_idle[MAXP];
static int npcpus;
static unsigned long long prev_time;

int update_pcpu_stats(const struct timeval *now, unsigned int delay)
{
	const struct xenstat_node *n = &nodes[next_node - 1];
	unsigned long long t = (unsigned long long)now->tv_sec * 1000000ULL + now->tv_usec;
	int i, detected = n->np < 128 ? n->np : 128;
	(void)delay;

	if (detected > npcpus) {
		for (i = 0; i < detected; i++) {
			prev_idle[i] = n->idle[i] / 1000;
			usage[i] = 0.0;
		}
		npcpus = detected;
		prev_time = t;
		return 0;
	}
	for (i = 0; i < detected; i++) {
		unsigned long long cur = n->idle[i] / 1000;
		unsigned long long diff = cur - prev_idle[i];
		if (t - prev_time > 0) {
			double u = 100.0 * (1.0 - ((double)diff / (t - prev_time)));
			usage[i] = u < 0 ? 0.0 : u > 100 ? 100.0 : u;
		} else {
			usage[i] = 0.0;
		}
		prev_idle[i] = cur;
	}
	prev_time = t;
	return 0;
}
int get_pcpu_count(void) { return npcpus; }
float get_pcpu_usage(int i) { return i >= 0 && i < npcpus ? usage[i] : 0.0; }
int has_pcpu_data(void) { return npcpus > 0; }
void free_pcpu_stats(void) { }
