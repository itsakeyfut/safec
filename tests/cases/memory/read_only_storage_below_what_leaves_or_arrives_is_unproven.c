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
void rows(char *const (*a)[2]);
void handed_rows_of_const_pointers(void) {
    char **b = malloc(2 * sizeof(char *));
    if (b) {
        b[0] = "ab";
        b[1] = "cd";
        rows((char *const (*)[2])b);
    }
}
void arrival_through_void(const int **pp) {
    if (pp) {
        void *v = pp;
        int **q = v;
        int *p = *q;
        if (p) {
            *p = 2;
        }
    }
}
void implicit_conversion_of_a_result(void) {
    void *v = result();
    int **q = v;
    if (q) {
        int *p = *q;
        if (p) {
            *p = 2;
        }
    }
}
char *find(const char *s);
int found_in_a_literal(void) {
    char *m = find("ab");
    if (m) {
        *m = 'x';
    }
    return 0;
}
void *realloc(void *p, unsigned long long n);
const char **get_names(void);
void copied_by_realloc(void) {
    const char **pp = get_names();
    if (!pp) {
        return;
    }
    char **q = realloc(pp, 8);
    if (!q) {
        return;
    }
    char *r = *q;
    if (r) {
        *r = 'x';
    }
}
char **GW;
void read_through_a_global_alias(const char **pp) {
    char **q = GW;
    if (pp && q) {
        char *s = *q;
        if (s) {
            *s = 'x';
        }
    }
}
void get_into(const int **out);
void written_through_a_const_argument(void) {
    int *a[1];
    get_into((const int **)a);
    int *p = a[0];
    if (p) {
        *p = 2;
    }
}
char *pick(const char **pp);
int found_one_load_down(void) {
    const char *p = "ab";
    char *m = pick(&p);
    if (m) {
        *m = 'x';
    }
    return 0;
}
void handed_unprototyped_through_a_cast(void) {
    const char *c = "ab";
    unp((char **)&c);
}
void take_any(void *a);
void chained_through_void_slots(void) {
    void *y[1];
    void *x[1];
    y[0] = "ab";
    x[0] = y;
    take_any(x);
}
void stored_through_a_const_lvalue_cast(char **out) {
    if (out) {
        *(const char **)out = "ab";
    }
}
void copied_into_a_const_out_parameter(const char **out) {
    const char *c = "ab";
    if (out) {
        memcpy(out, &c, sizeof c);
    }
}
void arrived_three_deep_handed_as_void(const char ***pp) {
    take_any(pp);
}
char **GW2;
void stored_through_a_global_beside_a_const_parameter(const char **unused) {
    char **q = GW2;
    if (q) {
        *q = "ab";
    }
}
void get_v(void *out);
void written_through_a_void_cast_argument(void) {
    int *a[1];
    a[0] = 0;
    const int **ca = (const int **)a;
    get_v((void *)ca);
    int *p = a[0];
    if (p) {
        *p = 2;
    }
}
const char *CG2[2];
void stored_into_a_const_static_through_a_cast(void) {
    *(char **)CG2 = "ab";
}
char *GC;
void copied_into_a_static_through_a_const_cast(void) {
    const char *s = "ab";
    memcpy((const char **)&GC, &s, sizeof s);
}
void copied_below_a_parameter_through_a_const_cast(char ***out) {
    if (out) {
        char **r = *out;
        if (r) {
            const char *s = "ab";
            memcpy((const char **)r, &s, sizeof s);
        }
    }
}
