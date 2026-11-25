#define STR(x) #x
#define DROP(y)
#define F(x) DROP(x)
F(_Pragma(STR(GCC warning "dropped")))
#define TWICE(x) x x
TWICE(_Pragma("GCC warning \"twice\""))

int main(void) { return 0; }
