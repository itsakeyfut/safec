void *malloc(int n);
void free(void *p);
int cond(void);

int main(void) {
    int *a = malloc(4);
    if (a == 0) {
        return 0;
    }
    int *b = malloc(4);
    if (b == 0) {
        return 0;
    }
    a[0] = 1;
    b[0] = 1;
    int **t2 = &a;
    if (cond()) {
        t2 = &b;
    }
    free(a);
    int *q = *t2;
    return *q;
}
