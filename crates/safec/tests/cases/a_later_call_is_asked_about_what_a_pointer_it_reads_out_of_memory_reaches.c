void *malloc(int n);
int release(int *p);

int main(void) {
    int x;
    int **s = malloc(8);
    int *a = malloc(4);
    if (s == 0) {
        return 0;
    }
    if (a == 0) {
        return 0;
    }
    *s = a;
    a[0] = 1;
    return (x = a[0]) + (release(*s), 0);
}
