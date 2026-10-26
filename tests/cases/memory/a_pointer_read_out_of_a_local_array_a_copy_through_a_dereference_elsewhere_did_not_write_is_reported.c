void *memcpy(void *s1, void *s2, int n);
int main(void) {
    int *m[1];
    int **p = m;
    int a = 0;
    int b = 1;
    int *pa = &a;
    int **ppa = &pa;
    memcpy(*ppa, &b, 4);
    int *q = *p;
    if (q == 0) { return 1; }
    *q = 1;
    return a;
}
