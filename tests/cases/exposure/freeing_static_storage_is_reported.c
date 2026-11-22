void free(void *p);
void *malloc(unsigned long long n);
void *realloc(void *p, unsigned long long n);
int g;
void freed_by_address(void) {
    free(&g);
}
void freed_on_one_path(int c) {
    int *p;
    if (c) {
        p = &g;
    } else {
        p = malloc(4);
    }
    free(p);
}
void reallocated(void) {
    void *q = realloc(&g, 8);
    free(q);
}
void freed_or_null(int c) {
    int *p = 0;
    if (c) {
        p = &g;
    }
    free(p);
}
void freed_on_one_path_or_converted(int c, long long n) {
    int *p;
    if (c) {
        p = &g;
    } else {
        p = (int *)n;
    }
    free(p);
}
