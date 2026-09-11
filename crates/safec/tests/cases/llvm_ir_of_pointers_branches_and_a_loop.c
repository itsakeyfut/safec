void bump(int *p) {
    *p = *p + 1;
}

int main() {
    int n;
    int *p;
    char c;
    n = 40;
    p = &n;
    bump(p);
    c = 2;
    if (n > 40) {
        c = c + 1;
    }
    while (c < 5) {
        c = c + 1;
    }
    return *p + c;
}
