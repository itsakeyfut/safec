void *malloc(int n);

__attribute__((annotate("safec_unchecked")))
void nothing(void) {
}

int main(void) {
    int x;
    int *a = malloc(4);
    if (a == 0) {
        return 0;
    }
    a[0] = 1;
    return (x = a[0]) + (nothing(), 0);
}
