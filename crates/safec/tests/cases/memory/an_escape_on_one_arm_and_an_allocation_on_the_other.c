void *malloc(int n);
void free(void *p);
int f(int c) {
    int *q = malloc(4);
    int *p;
    int **pp;
    if (c) { pp = &p; *pp = q; } else { p = malloc(8); }
    free(q);
    free(p);
    return 0;
}
