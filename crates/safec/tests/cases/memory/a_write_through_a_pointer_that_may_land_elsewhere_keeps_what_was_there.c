void *malloc(int n);
void free(void *p);
int f(int c) {
    int *a = malloc(4);
    int *p = a;
    int *r;
    int *q = malloc(8);
    int **pp = &r;
    if (c) { pp = &p; }
    *pp = q;
    free(p);
    *a = 1;
    return 0;
}
