void *malloc(int n);
void free(void *p);
int f(int **pp) {
    int *q = malloc(8);
    int *z;
    *pp = q;
    free(z);
    return 0;
}
