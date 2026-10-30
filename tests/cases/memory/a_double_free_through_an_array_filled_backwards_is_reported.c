void *malloc(int n);
void free(void *p);
int main(void) {
    int *m[2];
    int **e = m + 2;
    int *a = malloc(4);
    while (e != m) {
        e = e - 1;
        *e = a;
    }
    free(a);
    int *q = m[0];
    free(q);
    return 0;
}
