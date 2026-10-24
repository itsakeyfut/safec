void *malloc(int n);
void free(void *p);
void get(int **pp);

int f(void) {
    int *p;
    get(&p);
    free(p);
    return 0;
}
