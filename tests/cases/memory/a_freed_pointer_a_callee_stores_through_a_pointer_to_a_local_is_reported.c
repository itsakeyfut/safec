void *malloc(int n);
void free(void *p);
void put(int ** _Nonnull pp) { int *x = malloc(4); if (x == 0) { return; } free(x); *pp = x; }
int main(void) {
    int *q;
    int **pq = &q;
    put(pq);
    int *r = *pq;
    if (r == 0) { return 1; }
    return *r;
}
