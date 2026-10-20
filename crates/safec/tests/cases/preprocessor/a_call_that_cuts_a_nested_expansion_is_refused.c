void *malloc(int n);
int release(int *p);
#define B release
#define A (x = *p) + B

int main(void) {
    int *p = malloc(4);
    if (!p) return 0;
    int x;
    int y = A(p);
    return y;
}
