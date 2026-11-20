int main(void) {
    int x = 1;
    long long n = (long long)&x;
    int *q = (int *)n;
    return *q;
}
