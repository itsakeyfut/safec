void *memset(void *d, int c, unsigned long long n);
void *memcpy(void *d, const void *s, unsigned long long n);
void *memchr(const void *s, int c, unsigned long long n);
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
