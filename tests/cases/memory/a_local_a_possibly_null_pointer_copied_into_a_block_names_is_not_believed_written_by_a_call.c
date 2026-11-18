void *malloc(int n);
void *memcpy(void *d, void *s, int n);
void fill(int ***b);
int f(int c) {
    int *a;
    int **p = 0;
    if (c) p = &a;
    int ***b = malloc(8);
    if (!b) return 0;
    memcpy(b, &p, 8);
    fill(b);
    return a != 0;
}
