void *malloc(int n);
int h(int *p);

int main(void) {
    int *a = malloc(8);
    int x;
    if (a == 0) {
        return 0;
    }
    a[0] = 0;
    x = h(a + a[0]);
    return x;
}
