void *malloc(int n);
void free(void *p);
int main(void) {
    int *z = malloc(4);
    int *q = malloc(8);
    int **pp = &z;
    pp = pp + 1;
    pp = pp - 1;
    *pp = q;
    free(z);
    *q = 1;
    return 0;
}
