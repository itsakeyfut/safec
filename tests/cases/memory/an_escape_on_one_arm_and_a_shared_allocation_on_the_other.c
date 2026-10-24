void *malloc(int n);
void free(void *p);
int f(int c) {
    int *p;
    int *q;
    int **pp;
    if (c) { pp = &p; } else { p = malloc(4); q = p; }
    free(q);
    return 0;
}
