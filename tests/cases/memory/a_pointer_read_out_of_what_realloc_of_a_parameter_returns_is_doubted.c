// A caller may pass a null, which `realloc` reads as `malloc`, so what
// it returns may be indeterminate (#931).
void *realloc(void *p, int n);
void free(void *p);
int g(int **old) {
    int **pp = realloc(old, 8);
    if (!pp) { free(old); return 1; }
    int *p = *pp;
    free(pp);
    return p != 0;
}
