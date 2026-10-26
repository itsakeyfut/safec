void *memcpy(void *s1, void *s2, int n);

int main(void) {
    int x = 0;
    int *p = &x;
    int *q;
    int **pq = &q;
    memcpy(pq, &p, 8);
    int *r = *pq;
    return r == 0;
}
