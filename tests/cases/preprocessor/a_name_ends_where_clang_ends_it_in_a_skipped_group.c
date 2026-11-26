void *malloc(unsigned long long);
void free(void *);

int incomplete(void) {
    int *p = malloc(sizeof *p);
    if (!p) return 1;
    *p = 1;
    free(p);
#if 0
#else\u00e
    return *p;
#endif
    return 0;
}

int universal_space(void) {
    int *p = malloc(sizeof *p);
    if (!p) return 1;
    *p = 1;
    free(p);
#if 0
#else\u00a0
    return *p;
#endif
    return 0;
}

int mongolian_separator(void) {
    int *p = malloc(sizeof *p);
    if (!p) return 1;
    *p = 1;
    free(p);
#if 0
#else᠎
    return *p;
#endif
    return 0;
}

