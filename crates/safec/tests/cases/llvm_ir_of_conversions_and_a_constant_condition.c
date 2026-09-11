int narrow(char c) {
    return c;
}

int mix(int a, char b) {
    return a + b;
}

int main() {
    int r;
    char c;
    r = 0;
    c = 300;
    if (1) {
        r = r + narrow(300);
    }
    r = r + c;
    r = r + mix(1, 260);
    c = r;
    return c;
}
