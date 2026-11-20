int main(void) {
    int x = 1;
    const long long *q = &x;
    const int *r = &x;
    return *q != *r;
}
