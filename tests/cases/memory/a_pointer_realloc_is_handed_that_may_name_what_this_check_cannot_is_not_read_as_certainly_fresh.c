void *realloc(void *p, int n);
void free(void *p);
void unknown(int ***p);

int read_out_of_memory(int *** _Nonnull q) {
    int **o = *q;
    int **pp = realloc(o, 8);
    if (!pp) return 1;
    int *p = *pp;
    free(pp);
    return p == 0;
}

int written_by_a_call(void) {
    int **o = 0;
    unknown(&o);
    int **pp = realloc(o, 8);
    if (!pp) return 1;
    int *p = *pp;
    free(pp);
    return p == 0;
}
