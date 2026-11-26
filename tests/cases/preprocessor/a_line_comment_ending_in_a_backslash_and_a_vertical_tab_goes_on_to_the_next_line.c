void *malloc(unsigned long long);
void free(void *);

int in_a_comment(void) {
    int *p = malloc(sizeof *p);
    if (!p) return 1;
    free(p);
    // done \
    return 0;
    free(p);
    return 0;
}
