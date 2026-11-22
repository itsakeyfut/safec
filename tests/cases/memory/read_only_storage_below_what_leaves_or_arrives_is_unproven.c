void *malloc(unsigned long long n);
const int k = 1;
void set2(int **pp) {
    if (pp) {
        int *p = *pp;
        if (p) {
            *p = 2;
        }
    }
}
void handed_below(void) {
    int *c = (int *)&k;
    set2(&c);
}
int **returned_block(void) {
    int **b = malloc(sizeof(int *));
    if (b) {
        *b = (int *)&k;
    }
    return b;
}
void out_parameter(int **out) {
    if (out) {
        *out = (int *)&k;
    }
}
char *G;
void into_a_static(void) {
    G = "ab";
}
void cast_after_arrival(const int **pp) {
    if (pp) {
        int **q = (int **)pp;
        int *p = *q;
        if (p) {
            *p = 2;
        }
    }
}
void three_levels_arrived(const char ***ppp) {
    if (ppp) {
        char ***q = (char ***)ppp;
        char **r = *q;
        if (r) {
            char *s = *r;
            if (s) {
                *s = 'x';
            }
        }
    }
}
int read_only_after_arrival(const int **pp) {
    if (pp) {
        const int *p = *pp;
        if (p) {
            return *p;
        }
    }
    return 0;
}
void reads(const int **pp);
void handed_to_a_reader(void) {
    const int *c = &k;
    reads(&c);
}
char buf[3];
char *H;
void a_writable_one_leaves(void) {
    H = buf;
}
const int **result(void);
void cast_after_a_result(void) {
    int **q = (int **)result();
    if (q) {
        int *p = *q;
        if (p) {
            *p = 2;
        }
    }
}
const char *CG;
void into_a_const_static(void) {
    CG = "ab";
}
void unp();
void handed_unprototyped(void) {
    const char *p = "ab";
    unp(&p);
}
void *returned_as_void(void) {
    int **b = malloc(sizeof(int *));
    if (b) {
        *b = (int *)&k;
    }
    return b;
}
void *memcpy(void *d, const void *s, unsigned long long n);
void copied_out(char **out) {
    char *c = "ab";
    if (out) {
        memcpy(out, &c, sizeof c);
    }
}
const char *CA[2];
const char *CA2[2][2];
void into_a_const_static_array(void) {
    CA[0] = "ab";
    CA2[1][0] = "ab";
}
char *WA[2];
void into_a_writable_static_array(void) {
    WA[0] = "ab";
}
