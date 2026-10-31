void *memcpy(void *d, void *s, int n);
int main(void) {
    int x = 0;
    int *p = &x;
    int *slot;
    int **pp = &slot;
    pp = pp + 1;
    pp = pp - 1;
    memcpy(pp, &p, 8);
    int *q = slot;
    return q == &x;
}
