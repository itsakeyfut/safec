void *memset(void *d, int c, unsigned long long n);
void *memcpy(void *d, const void *s, unsigned long long n);
void *memchr(const void *s, int c, unsigned long long n);
void *malloc(unsigned long long n);
extern const int e;
int h;
const int a[2];
const int k = 1;
void set(int *p) {
    if (p) {
        *p = 2;
    }
}
void through_its_address(void) {
    int *p = (int *)&a;
    *p = 1;
}
void read_back_out_of_a_local(void) {
    const int *c = &k;
    int *q = *(int **)&c;
    if (q) {
        *q = 3;
    }
}
void read_back_through_void(void) {
    const int *c = &k;
    void *v = &c;
    int **pp = v;
    int *q = *pp;
    if (q) {
        *q = 3;
    }
}
void a_const_local(void) {
    const int x = 1;
    const int *c = &x;
    int *q = *(int **)&c;
    if (q) {
        *q = 3;
    }
}
void filled(void) {
    memset(&a, 0, sizeof a);
}
void copied_by_bytes_and_handed_on(void) {
    const int *cp = &k;
    int *p;
    memcpy(&p, &cp, sizeof p);
    set(p);
}
void found_by_memchr(void) {
    int *p = memchr(&k, 1, sizeof k);
    if (p) {
        *p = 2;
    }
}
void joined(int n) {
    const int *c;
    if (n) {
        c = &k;
    } else {
        c = &h;
    }
    int *q = *(int **)&c;
    if (q) {
        *q = 3;
    }
}
void stored_on_one_path(int n) {
    const int **box = malloc(sizeof(const int *));
    if (!box) {
        return;
    }
    if (n) {
        *box = &k;
    } else {
        *box = &h;
    }
    int *q = *(int **)box;
    if (q) {
        *q = 3;
    }
}
void moved_by_arithmetic(void) {
    const int *c = a;
    int *p = *(int **)&c;
    p = p + 1;
    *p = 3;
}
void a_const_local_handed_on(void) {
    const int x = 1;
    const int *c = &x;
    int *q = *(int **)&c;
    set(q);
}
void handed_one_level_down(void) {
    const int x = 1;
    const int *c = &x;
    int **pp = (int **)&c;
    set(*pp);
}
void a_const_parameter(const int n) {
    const int *c = &n;
    int *q = *(int **)&c;
    if (q) {
        *q = 1;
    }
}
void an_extern_const(void) {
    const int *c = &e;
    int *q = *(int **)&c;
    if (q) {
        *q = 1;
    }
}
void a_block_extern_const(void) {
    extern const int only_here;
    const int *c = &only_here;
    int *q = *(int **)&c;
    if (q) {
        *q = 1;
    }
}
