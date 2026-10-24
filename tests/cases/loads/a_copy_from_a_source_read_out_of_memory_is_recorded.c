void *malloc(int n);
void free(void *p);
void refill(int **slot);
void release(int *p);
void *memcpy(void *d, void *s, int n);

int main(void) {
    int **tab = malloc(8);
    if (tab == 0) {
        return 0;
    }
    int *p = malloc(4);
    if (p == 0) {
        return 0;
    }
    int *r = malloc(4);
    if (r == 0) {
        return 0;
    }
    p[0] = 1;
    r[0] = 2;
    int **src = malloc(8);
    if (src == 0) {
        return 0;
    }
    int ***ps = malloc(8);
    if (ps == 0) {
        return 0;
    }
    *tab = p;
    *src = r;
    *ps = src;
    memcpy(tab, *ps, 8);
    free(r);
    int *q = *tab;
    if (q == 0) {
        return 0;
    }
    return *q;
}
