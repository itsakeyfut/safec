void *malloc(unsigned long long);
void free(void *);

int in_a_macro_body(void) {
    int *p = malloc(sizeof *p);
    if (!p) return 1;
    free(p);
#define NOTE \
    p = malloc(sizeof *p);
    free(p);
    return 0;
}

int in_a_skipped_directive(void) {
    int *p = malloc(sizeof *p);
    if (!p) return 1;
    free(p);
#if 0
#define X \
#else
    return 0;
#endif
    free(p);
    return 0;
}
