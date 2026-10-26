void *memcpy(void *s1, void *s2, int n);

int main(void) {
    int x = 0;
    int *src[1];
    src[0] = &x;
    int *m[1];
    int **p = m;
    int ***pp = &p;
    memcpy(*pp, src, 8);
    int *q = m[0];
    return q == 0;
}
