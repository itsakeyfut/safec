int narrow(char c) {
    return c;
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
    c = r;
    return c;
}
