void *malloc(int n);
int keep2(int v, int *p);
int release_all(void);

int main(void) {
    int r;
    int *a = malloc(4);
    if (a == 0) {
        return 0;
    }
    a[0] = 1;
    r = keep2(a[0], a) + release_all();
    return r;
}
