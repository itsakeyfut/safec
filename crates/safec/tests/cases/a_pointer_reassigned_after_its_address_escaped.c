void *malloc(int n);
void free(void *p);
int f(void) {
    int *p = malloc(4);
    int **pp = &p;
    int *q = malloc(4);
    p = malloc(8);
    free(q);
    *pp = q;
    *p = 1;
    return 0;
}
