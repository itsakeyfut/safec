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
    *tab = p;
    *src = r;
    memcpy(tab, src, 8);
    r[0] = 3;
    int *q = *tab;
    if (q == 0) {
        return 0;
    }
    return *q;
}

int deep(void) {
    int ***t3 = malloc(8);
    if (t3 == 0) {
        return 0;
    }
    int **t2 = malloc(8);
    if (t2 == 0) {
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
    r[0] = 1;
    *t2 = p;
    *t3 = t2;
    **t3 = r;
    r[0] = 3;
    return ***t3;
}
