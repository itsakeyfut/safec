void *malloc(int n);
int release(int *p);
#define F (x = *p) + release

int main(void) {
    int *p = malloc(4);
    if (!p) return 0;
    int x;
    int y = F(p);
    return y;
}
