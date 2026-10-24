void *malloc(int n);
void free(void *p);
int f(int c) {
    int *p = malloc(4);
    int *r = p;
    int **tmp = &p;
    tmp = 0;
    int *s;
    int **pp;
    if (c) {
        pp = &s;
    }
    int *q = malloc(8);
    *pp = q;
    free(p);
    *r = 1;
    return 0;
}
