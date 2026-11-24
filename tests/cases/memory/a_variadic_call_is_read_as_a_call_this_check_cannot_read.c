void *malloc(unsigned long long);
void free(void *);
int printf(const char *format, ...);
int keep(int *_Nonnull p, ...);

int read_after_free(void) {
    int *p = malloc(4);
    if (!p)
        return 0;
    *p = 1;
    free(p);
    printf("%d", *p);
    return 0;
}

int handed_after_free(void) {
    int *p = malloc(4);
    if (!p)
        return 0;
    free(p);
    printf("%p", p);
    return 0;
}

int null_to_the_nonnull_parameter(void) {
    int x = 1;
    keep(0, &x, 2);
    return x;
}

int handed_then_read(void) {
    int *p = malloc(4);
    if (!p)
        return 0;
    *p = 1;
    keep(p, p);
    int v = *p;
    free(p);
    return v;
}

int defined(int a, ...) {
    return a;
}
