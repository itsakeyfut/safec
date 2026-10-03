void *malloc(int n);
void free(void *p);
int f(void) {
    int *p = malloc(4);
    int *q = malloc(8);
    int **pp = &p;
    *pp = q;
    free(p);
    *q = 1;
    return 0;
}
