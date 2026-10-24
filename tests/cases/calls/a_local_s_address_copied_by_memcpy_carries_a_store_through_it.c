void *malloc(int n);
void free(void *p);
void *memcpy(void *d, void *s, int n);
int f(void) {
    int *slot = 0;
    int **p = &slot;
    int **s = 0;
    int *r = malloc(4);
    if (r == 0) {
        return 0;
    }
    memcpy(&s, &p, 8);
    if (s == 0) {
        return 0;
    }
    *s = r;
    free(r);
    if (slot == 0) {
        return 0;
    }
    return *slot;
}
